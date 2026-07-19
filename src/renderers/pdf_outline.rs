//! PDF 后处理模块 — 书签/大纲生成 + PDF 元数据写入
//!
//! 修复要点：
//! 1. 使用 doc.add_object() 分配 ID，避免手动分配冲突
//! 2. 通过 trailer → Root → objects 手动更新 Catalog，不依赖 catalog_mut()
//! 3. 增强标题提取：同时支持 .header 锚点和直接 h1-h6 扫描
//! 4. 正确计算 /Count（所有后代总数）
//! 5. 完善中文 UTF-16BE 编码

use indexmap::IndexMap;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use scraper::{Html, Selector};
use std::path::Path;

// ═══════════════════════════════════════════════════════════
// 公开接口
// ═══════════════════════════════════════════════════════════

/// 书签条目
#[derive(Debug, Clone)]
pub struct BEntry {
    pub level: u32,
    pub title: String,
    pub dest_name: String,
}

/// 对生成的 PDF 执行后处理：书签 + 元数据
pub fn postprocess_pdf(
    pdf_path: &Path,
    html_content: &str,
    title: Option<&str>,
    author: Option<&str>,
    language: Option<&str>,
) -> Result<(), anyhow::Error> {
    let mut doc = Document::load(pdf_path)
        .map_err(|e| anyhow::anyhow!("无法加载 PDF: {}", e))?;

    // ── 1. 提取书签条目 ──
    let entries = extract_bookmark_entries(html_content)?;
    log::info!(
        "mdbook-pdf(outline): 从 HTML 提取了 {} 个书签条目",
        entries.len()
    );

    // ── 2. 解析命名目标 ──
    let dest_map = resolve_dests(&doc);
    log::info!(
        "mdbook-pdf(outline): PDF 中解析了 {} 个命名目标",
        dest_map.len()
    );

    // ── 3. 添加书签 ──
    let mut outline_id: Option<ObjectId> = None;
    if !entries.is_empty() {
        match add_bookmarks(&mut doc, &entries, &dest_map) {
            Ok(Some(oid)) => {
                outline_id = Some(oid);
                log::info!("mdbook-pdf(outline): 成功添加 {} 个书签", entries.len());
            }
            Ok(None) => {
                log::warn!("mdbook-pdf(outline): add_bookmarks 返回 None");
            }
            Err(e) => {
                log::warn!("mdbook-pdf(outline): 添加书签失败: {}", e);
            }
        }
    } else {
        log::warn!("mdbook-pdf(outline): 未提取到任何书签条目，跳过");
    }

    // ── 4. 写入元数据 ──
    let metadata_id = add_metadata(&mut doc, title, author, language);

    // ── 5. 更新 Catalog（关键修复：不依赖 catalog_mut）──
    update_catalog(&mut doc, outline_id, metadata_id, language)?;

    // ── 6. 保存 ──
    doc.save(pdf_path)
        .map_err(|e| anyhow::anyhow!("无法保存 PDF: {}", e))?;

    log::info!("mdbook-pdf(outline): 后处理完成");
    Ok(())
}

// ═══════════════════════════════════════════════════════════
// 书签提取（增强版：支持多种 HTML 格式）
// ═══════════════════════════════════════════════════════════

/// 从 HTML 提取书签条目
///
/// 策略（按优先级）：
/// 1. 查找 `<a class="header" href="#id">` → 对应 h1-h6（mdbook 标准格式）
/// 2. 回退：直接扫描所有 h1-h6[id] 元素（兼容无锚点格式）
pub fn extract_bookmark_entries(html: &str) -> Result<Vec<BEntry>, anyhow::Error> {
    let doc = Html::parse_document(html);
    let mut entries = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    // ── 策略 1：通过 .header 锚点查找 ──
    if let Ok(header_sel) = Selector::parse("a.header[href]") {
        for el in doc.select(&header_sel) {
            let href = match el.value().attr("href") {
                Some(h) if h.starts_with('#') => &h[1..],
                _ => continue,
            };
            if seen_ids.contains(href) {
                continue;
            }

            // 查找对应 id 的标题元素
            if let Some(entry) = find_heading_by_id(&doc, href) {
                seen_ids.insert(href.to_string());
                entries.push(entry);
            }
        }
    }

    // ── 策略 2：回退 — 直接扫描 h1-h6[id] ──
    if entries.is_empty() {
        log::info!("mdbook-pdf(outline): .header 锚点未匹配，回退到直接扫描 h1-h6");
        if let Ok(heading_sel) = Selector::parse("h1[id], h2[id], h3[id], h4[id], h5[id], h6[id]") {
            for el in doc.select(&heading_sel) {
                let id = match el.value().attr("id") {
                    Some(id) if !id.is_empty() => id,
                    _ => continue,
                };
                if seen_ids.contains(id) {
                    continue;
                }

                let tag = el.value().name();
                let level = tag_to_level(tag);
                if level == 0 {
                    continue;
                }

                let title: String = el.text().collect::<String>().trim().to_string();
                if title.is_empty() {
                    continue;
                }

                seen_ids.insert(id.to_string());
                entries.push(BEntry {
                    level,
                    title,
                    dest_name: id.to_string(),
                });
            }
        }
    }

    Ok(entries)
}

/// 在文档中查找指定 id 的标题元素
fn find_heading_by_id(doc: &Html, target_id: &str) -> Option<BEntry> {
    // 使用属性选择器（避免 CSS ID 选择器对中文/特殊字符的解析问题）
    let escaped = target_id.replace('\\', "\\\\").replace('"', "\\\"");
    let sel_str = format!("[id=\"{}\"]", escaped);
    let sel = Selector::parse(&sel_str).ok()?;

    let el = doc.select(&sel).next()?;
    let tag = el.value().name();
    let level = tag_to_level(tag);
    if level == 0 {
        return None;
    }

    let title: String = el.text().collect::<String>().trim().to_string();
    if title.is_empty() {
        return None;
    }

    Some(BEntry {
        level,
        title,
        dest_name: target_id.to_string(),
    })
}

fn tag_to_level(tag: &str) -> u32 {
    match tag.to_lowercase().as_str() {
        "h1" => 1,
        "h2" => 2,
        "h3" => 3,
        "h4" => 4,
        "h5" => 5,
        "h6" => 6,
        _ => 0,
    }
}

// ═══════════════════════════════════════════════════════════
// 命名目标解析
// ═══════════════════════════════════════════════════════════

fn resolve_dests(doc: &Document) -> IndexMap<String, ObjectId> {
    let mut map = IndexMap::new();
    let catalog = match doc.catalog() {
        Ok(c) => c,
        Err(_) => return map,
    };

    // 方式 1: /Dests 直接字典
    if let Ok(dests_obj) = catalog.get(b"Dests") {
        let dests_obj = resolve_ref(doc, dests_obj);
        if let Ok(dict) = dests_obj.as_dict() {
            for (name_bytes, dest_obj) in dict.iter() {
                let name = percent_decode_to_lossy(name_bytes);
                if let Some(page_id) = dest_obj_to_page_id(doc, dest_obj) {
                    map.insert(name, page_id);
                }
            }
        }
    }

    // 方式 2: /Names → /Dests 名称树
    if let Ok(names_obj) = catalog.get(b"Names") {
        let names_obj = resolve_ref(doc, names_obj);
        if let Ok(names_dict) = names_obj.as_dict() {
            if let Ok(dests_obj) = names_dict.get(b"Dests") {
                collect_name_tree(doc, dests_obj, &mut map);
            }
        }
    }

    map
}

fn resolve_ref<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
    match obj {
        Object::Reference(id) => doc.get_object(*id).map(|o| resolve_ref(doc, o)).unwrap_or(obj),
        _ => obj,
    }
}

fn collect_name_tree(doc: &Document, obj: &Object, map: &mut IndexMap<String, ObjectId>) {
    match obj {
        Object::Reference(id) => {
            if let Ok(inner) = doc.get_object(*id) {
                collect_name_tree(doc, inner, map);
            }
        }
        Object::Dictionary(dict) => {
            // /Names 数组：[name1, dest1, name2, dest2, ...]
            if let Ok(names) = dict.get(b"Names") {
                if let Ok(arr) = names.as_array() {
                    for chunk in arr.chunks(2) {
                        if chunk.len() == 2 {
                            let name_bytes = chunk[0]
                                .as_name().ok().map(|n| n.to_vec())
                                .or_else(|| chunk[0].as_string().ok().map(|s| s.as_bytes().to_vec()));
                            if let Some(nb) = name_bytes {
                                let name = percent_decode_to_lossy(&nb);
                                if let Some(pid) = dest_obj_to_page_id(doc, &chunk[1]) {
                                    map.insert(name, pid);
                                }
                            }
                        }
                    }
                }
            }
            // /Kids 或 /Children 递归
            for key in [b"Kids".as_ref(), b"Children".as_ref()] {
                if let Ok(children) = dict.get(key) {
                    if let Ok(arr) = children.as_array() {
                        for child in arr {
                            collect_name_tree(doc, child, map);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn dest_obj_to_page_id(doc: &Document, obj: &Object) -> Option<ObjectId> {
    match obj {
        Object::Array(arr) => arr.first()?.as_reference().ok(),
        Object::Reference(id) => {
            let inner = doc.get_object(*id).ok()?;
            dest_obj_to_page_id(doc, inner)
        }
        Object::Dictionary(dict) => {
            let d = dict.get(b"D").ok()?;
            dest_obj_to_page_id(doc, d)
        }
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════
// 书签构建（修复版：使用 add_object 分配 ID）
// ═══════════════════════════════════════════════════════════

fn add_bookmarks(
    doc: &mut Document,
    entries: &[BEntry],
    dest_map: &IndexMap<String, ObjectId>,
) -> Result<Option<ObjectId>, anyhow::Error> {
    if entries.is_empty() {
        return Ok(None);
    }

    let total = entries.len();

    // ── 第一步：计算父子关系 ──
    let mut parent_of: Vec<Option<usize>> = vec![None; total];
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); total];
    let mut stack: Vec<(u32, usize)> = Vec::new();

    for (i, entry) in entries.iter().enumerate() {
        while let Some(&(top_level, _)) = stack.last() {
            if top_level < entry.level {
                break;
            }
            stack.pop();
        }
        if let Some(&(_, parent_idx)) = stack.last() {
            parent_of[i] = Some(parent_idx);
            children_of[parent_idx].push(i);
        }
        stack.push((entry.level, i));
    }

    let top_level: Vec<usize> = (0..total).filter(|&i| parent_of[i].is_none()).collect();
    if top_level.is_empty() {
        return Ok(None);
    }

    // ── 第二步：计算每个节点的后代总数（用于 /Count）──
    let mut descendant_count: Vec<i64> = vec![0; total];
    // 从后向前遍历（子节点一定在父节点之后）
    for i in (0..total).rev() {
        let mut count = children_of[i].len() as i64;
        for &child in &children_of[i] {
            count += descendant_count[child];
        }
        descendant_count[i] = count;
    }
    let total_descendants: i64 = top_level.iter().map(|&i| 1 + descendant_count[i]).sum();

    // ── 第三步：使用 doc.add_object() 安全分配 ID ──
    // 先创建占位字典，获取 ID，再填充内容
    let outline_id: ObjectId = doc.add_object(Dictionary::new());
    let mut item_ids: Vec<ObjectId> = Vec::with_capacity(total);
    for _ in 0..total {
        item_ids.push(doc.add_object(Dictionary::new()));
    }

    // ── 第四步：填充每个书签字典 ──
    for i in 0..total {
        let entry = &entries[i];
        let mut dict = Dictionary::new();

        // Title（支持中文）
        dict.set("Title", pdf_string(&entry.title));

        // Dest
        if let Some(&page_id) = dest_map.get(&entry.dest_name) {
            dict.set("Dest", Object::Array(vec![
                Object::Reference(page_id),
                Object::Name(b"FitH".to_vec()),
                Object::Null,
            ]));
        } else {
            // 回退：使用命名目标名称字符串
            dict.set("Dest", pdf_string(&entry.dest_name));
        }

        // 子节点链接
        let children = &children_of[i];
        if !children.is_empty() {
            dict.set("First", Object::Reference(item_ids[children[0]]));
            dict.set("Last", Object::Reference(item_ids[*children.last().unwrap()]));
            dict.set("Count", Object::Integer(descendant_count[i]));
        }

        // Parent
        let parent_id = match parent_of[i] {
            Some(pidx) => item_ids[pidx],
            None => outline_id,
        };
        dict.set("Parent", Object::Reference(parent_id));

        // Prev / Next（在同级兄弟中）
        let siblings: &[usize] = match parent_of[i] {
            Some(pidx) => &children_of[pidx],
            None => &top_level,
        };
        if let Some(pos) = siblings.iter().position(|&x| x == i) {
            if pos > 0 {
                dict.set("Prev", Object::Reference(item_ids[siblings[pos - 1]]));
            }
            if pos + 1 < siblings.len() {
                dict.set("Next", Object::Reference(item_ids[siblings[pos + 1]]));
            }
        }

        // 写入 doc.objects
        doc.objects.insert(item_ids[i], Object::Dictionary(dict));
    }

    // ── 第五步：构建大纲根字典 ──
    let mut outline_dict = Dictionary::new();
    outline_dict.set("Type", Object::Name(b"Outlines".to_vec()));
    outline_dict.set("First", Object::Reference(item_ids[top_level[0]]));
    outline_dict.set("Last", Object::Reference(item_ids[*top_level.last().unwrap()]));
    outline_dict.set("Count", Object::Integer(total_descendants));

    doc.objects.insert(outline_id, Object::Dictionary(outline_dict));

    Ok(Some(outline_id))
}

// ═══════════════════════════════════════════════════════════
// Catalog 更新（关键修复）
// ═══════════════════════════════════════════════════════════

/// 手动更新 Catalog 字典，不依赖 catalog_mut()
///
/// 修复原因：catalog_mut() 在 Chrome 生成的 PDF 中可能因
/// 对象流（ObjStm）或间接引用嵌套而返回 Err，导致 /Outlines
/// 永远不会被写入。
fn update_catalog(
    doc: &mut Document,
    outline_id: Option<ObjectId>,
    metadata_id: Option<ObjectId>,
    language: Option<&str>,
) -> Result<(), anyhow::Error> {
    // 从 trailer 获取 Root 的 ObjectId
    let root_id = doc
        .trailer
        .get(b"Root")
        .map_err(|_| anyhow::anyhow!("PDF trailer 中缺少 /Root"))?
        .as_reference()
        .map_err(|_| anyhow::anyhow!("/Root 不是间接引用"))?;

    // 获取 Catalog 字典的可变引用
    let catalog_obj = doc
        .objects
        .get_mut(&root_id)
        .ok_or_else(|| anyhow::anyhow!("Catalog 对象 {:?} 不存在", root_id))?;

    let catalog_dict = catalog_obj
        .as_dict_mut()
        .map_err(|_| anyhow::anyhow!("Catalog 对象不是字典"))?;

    if let Some(oid) = outline_id {
        catalog_dict.set("Outlines", Object::Reference(oid));
        log::debug!("mdbook-pdf(outline): Catalog /Outlines → {:?}", oid);
    }

    if let Some(mid) = metadata_id {
        catalog_dict.set("Metadata", Object::Reference(mid));
    }

    if let Some(lang) = language {
        if !lang.is_empty() {
            catalog_dict.set("Lang", pdf_string(lang));
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// PDF 元数据
// ═══════════════════════════════════════════════════════════

fn add_metadata(
    doc: &mut Document,
    title: Option<&str>,
    author: Option<&str>,
    language: Option<&str>,
) -> Option<ObjectId> {
    // ── Info 字典 ──
    let mut info = Dictionary::new();
    if let Some(t) = title {
        info.set("Title", pdf_string(t));
    }
    if let Some(a) = author {
        info.set("Author", pdf_string(a));
    }
    if let Some(l) = language {
        info.set("Lang", pdf_string(l));
    }
    info.set("Producer", pdf_string("mdbook-plugins pdf"));
    info.set("Creator", pdf_string("mdbook-plugins pdf"));

    let info_id = doc.add_object(Object::Dictionary(info));
    doc.trailer.set("Info", Object::Reference(info_id));

    // ── XMP 元数据流 ──
    let xmp = build_xmp(title, author);
    let mut xmp_dict = Dictionary::new();
    xmp_dict.set("Type", Object::Name(b"Metadata".to_vec()));
    xmp_dict.set("Subtype", Object::Name(b"XML".to_vec()));
    let xmp_stream = Stream::new(xmp_dict, xmp.into_bytes());
    let xmp_id = doc.add_object(xmp_stream);

    Some(xmp_id)
}

fn build_xmp(title: Option<&str>, author: Option<&str>) -> String {
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");

    let mut parts = vec![
        r#"<?xpacket begin="\u{FEFF}" id="W5M0MpCehiHzreSzNTczkc9d"?>"#.to_string(),
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">"#.to_string(),
        r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">"#.to_string(),
        r#"<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/">"#.to_string(),
    ];

    if let Some(t) = title {
        parts.push(format!(
            "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>",
            escape(t)
        ));
    }
    if let Some(a) = author {
        parts.push(format!(
            "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>",
            escape(a)
        ));
    }

    parts.push("</rdf:Description>".to_string());
    parts.push("</rdf:RDF>".to_string());
    parts.push("</x:xmpmeta>".to_string());
    parts.push(r#"<?xpacket end="w"?>"#.to_string());

    parts.join("\n")
}

// ═══════════════════════════════════════════════════════════
// PDF 字符串编码（中英文支持）
// ═══════════════════════════════════════════════════════════

/// 将字符串编码为 PDF 字符串对象
///
/// - 纯 ASCII → 字面字符串 `(text)`
/// - 含非 ASCII（中文等）→ UTF-16BE 十六进制 `<FEFF xxxx xxxx>`
fn pdf_string(text: &str) -> Object {
    if text.is_ascii() {
        Object::string_literal(text.to_string())
    } else {
        let mut buf: Vec<u8> = vec![0xFE, 0xFF]; // UTF-16BE BOM
        for code_unit in text.encode_utf16() {
            buf.extend_from_slice(&code_unit.to_be_bytes());
        }
        Object::String(buf, StringFormat::Hexadecimal)
    }
}

// ═══════════════════════════════════════════════════════════
// URL 解码
// ═══════════════════════════════════════════════════════════

fn percent_decode_to_lossy(bytes: &[u8]) -> String {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                decoded.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_html(body: &str) -> String {
        format!("<html><head><title>T</title></head><body>{}</body></html>", body)
    }

    // ── 书签提取 ──

    #[test]
    fn test_extract_with_header_class() {
        let html = make_html(
            r##"<a class="header" href="#intro"></a><h1 id="intro">Introduction</h1>
               <a class="header" href="#setup"></a><h2 id="setup">Setup</h2>"##,
        );
        let es = extract_bookmark_entries(&html).unwrap();
        assert_eq!(es.len(), 2);
        assert_eq!(es[0].level, 1);
        assert_eq!(es[0].title, "Introduction");
        assert_eq!(es[1].level, 2);
    }

    #[test]
    fn test_extract_fallback_direct_scan() {
        // 无 .header 锚点，回退到直接扫描 h1-h6[id]
        let html = make_html(
            r#"<h1 id="ch1">第一章</h1>
               <h2 id="ch1-s1">第一节</h2>
               <h1 id="ch2">Chapter 2</h1>"#,
        );
        let es = extract_bookmark_entries(&html).unwrap();
        assert_eq!(es.len(), 3);
        assert_eq!(es[0].title, "第一章");
        assert_eq!(es[0].level, 1);
        assert_eq!(es[1].title, "第一节");
        assert_eq!(es[2].title, "Chapter 2");
    }

    #[test]
    fn test_extract_chinese_titles() {
        let html = make_html(
            r##"<a class="header" href="#yin-yan"></a><h1 id="yin-yan">引言</h1>
               <a class="header" href="#an-zhuang"></a><h2 id="an-zhuang">安装指南</h2>"##,
        );
        let es = extract_bookmark_entries(&html).unwrap();
        assert_eq!(es.len(), 2);
        assert_eq!(es[0].title, "引言");
        assert_eq!(es[1].title, "安装指南");
    }

    #[test]
    fn test_extract_empty_html() {
        let es = extract_bookmark_entries("<html><body></body></html>").unwrap();
        assert!(es.is_empty());
    }

    // ── PDF 字符串编码 ──

    #[test]
    fn test_pdf_string_ascii() {
        let obj = pdf_string("Hello World");
        // 应该是字面字符串
        assert!(matches!(obj, Object::String(_, StringFormat::Literal)));
    }

    #[test]
    fn test_pdf_string_chinese() {
        let obj = pdf_string("你好世界");
        match &obj {
            Object::String(bytes, StringFormat::Hexadecimal) => {
                // 应以 BOM FE FF 开头
                assert_eq!(bytes[0], 0xFE);
                assert_eq!(bytes[1], 0xFF);
                // "你" = U+4F60 → 4F 60
                assert_eq!(bytes[2], 0x4F);
                assert_eq!(bytes[3], 0x60);
            }
            _ => panic!("Expected hexadecimal string for Chinese text"),
        }
    }

    #[test]
    fn test_pdf_string_mixed() {
        let obj = pdf_string("Chapter 1: 引言");
        // 含非 ASCII，应使用 UTF-16BE
        assert!(matches!(obj, Object::String(_, StringFormat::Hexadecimal)));
    }

    // ── URL 解码 ──

    #[test]
    fn test_percent_decode_ascii() {
        assert_eq!(percent_decode_to_lossy(b"hello"), "hello");
    }

    #[test]
    fn test_percent_decode_chinese() {
        // "引言" 的 UTF-8 编码: E5 BC 95 E8 A8 80
        assert_eq!(
            percent_decode_to_lossy(b"%E5%BC%95%E8%A8%80"),
            "引言"
        );
    }

    #[test]
    fn test_percent_decode_space() {
        assert_eq!(percent_decode_to_lossy(b"a%20b"), "a b");
    }

    // ── 标签层级 ──

    #[test]
    fn test_tag_to_level() {
        assert_eq!(tag_to_level("h1"), 1);
        assert_eq!(tag_to_level("h6"), 6);
        assert_eq!(tag_to_level("div"), 0);
        assert_eq!(tag_to_level("H2"), 2); // 大小写不敏感
    }
}