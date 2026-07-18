//! mdbook-pdf — PDF 后处理：书签（大纲）和元数据
//!
//! 替代原 Python `mdbook-pdf-outline` 脚本的功能。
//!
//! 在 Chrome CDP 生成 PDF 后进行后处理：
//! 1. 从 `print.html` 解析标题结构，生成 PDF 书签
//! 2. 写入 PDF 元数据
//!
//! 书签生成逻辑（"like-wkhtmltopdf" 模式）：
//! 查找 `<a class="header" href="#id">` 元素 → 找到对应 id 的标题元素
//! （h1, h2, h3...）→ 提取层级和标题文本 → 创建 PDF 书签。

use indexmap::IndexMap;
use lopdf::{Bookmark, Destination, Document, Object, ObjectId};
use scraper::{Html, Selector};
use std::path::Path;

use super::pdf::PdfOptions;

/// 对已生成的 PDF 进行后处理：添加书签和元数据
pub fn postprocess_pdf(
    pdf_path: &Path,
    print_html_path: &Path,
    cfg: &PdfOptions,
    book_title: &str,
    book_authors: &[String],
    book_description: &str,
    book_language: &str,
) -> Result<(), anyhow::Error> {
    let has_meta = !book_title.is_empty()
        || !book_authors.is_empty()
        || !book_description.is_empty()
        || !book_language.is_empty();

    if !cfg.generate_document_outline && !has_meta {
        return Ok(());
    }

    let mut doc = Document::load(pdf_path)
        .map_err(|e| anyhow::anyhow!("Failed to load PDF: {}", e))?;

    if cfg.generate_document_outline {
        if let Ok(html) = std::fs::read_to_string(print_html_path) {
            let entries = extract_bookmark_entries(&html)?;
            if !entries.is_empty() {
                add_bookmarks(&mut doc, &entries)?;
            }
        }
    }

    if has_meta {
        add_metadata(&mut doc, book_title, book_authors, book_description, book_language);
    }

    doc.save(pdf_path)
        .map_err(|e| anyhow::anyhow!("Failed to save PDF: {}", e))?;

    log::info!("mdbook-pdf(outline): post-processing complete");
    Ok(())
}

// ── 书签提取 ──

#[derive(Debug, Clone)]
struct BEntry {
    level: usize,
    title: String,
    dest_name: String,
}

/// 从 HTML 提取书签条目（查找 a.header + 对应 h1-h6 标题）
fn extract_bookmark_entries(html: &str) -> Result<Vec<BEntry>, anyhow::Error> {
    let doc = Html::parse_document(html);
    let mut entries = Vec::new();

    let sel = Selector::parse("a.header[href]")
        .map_err(|e| anyhow::anyhow!("Invalid selector: {}", e))?;

    for el in doc.select(&sel) {
        let href = match el.value().attr("href") {
            Some(h) if h.starts_with('#') => &h[1..],
            _ => continue,
        };

        // 查找对应 id 的元素
        let id_sel = Selector::parse(&format!("#{}", css_escape(href)))
            .map_err(|e| anyhow::anyhow!("Invalid selector: {}", e))?;

        let target = match doc.select(&id_sel).next() {
            Some(t) => t,
            None => continue,
        };

        // 从标签名获取层级：h1→1, h2→2...
        let tag = target.value().name();
        let level: usize = match tag.strip_prefix('h') {
            Some(n) => n.parse().unwrap_or(1),
            None => continue,
        };

        let title = target.text().collect::<String>().trim().to_string();
        if title.is_empty() {
            continue;
        }

        entries.push(BEntry { level, title, dest_name: href.to_string() });
    }

    Ok(entries)
}

// ── 书签写入 ──

fn add_bookmarks(doc: &mut Document, entries: &[BEntry]) -> Result<(), anyhow::Error> {
    // 获取命名目标 → 页面 ObjectId 映射
    let dest_map = resolve_dests(doc)?;

    // 构建书签树
    let mut stack: Vec<(usize, u32)> = Vec::new();

    for e in entries {
        let page_id = dest_map.get(&e.dest_name).copied().unwrap_or((0, 0));
        let bm = Bookmark::new(e.title.clone(), [0.0, 0.0, 0.0], 0, page_id);
        let parent = find_parent(&mut stack, e.level);
        let id = doc.add_bookmark(bm, parent);
        stack.push((e.level, id));
    }

    let _ = doc.build_outline();
    doc.adjust_zero_pages();

    log::info!("mdbook-pdf(outline): added {} bookmark(s)", entries.len());
    Ok(())
}

/// 从 PDF 中解析命名目标 → ObjectId 映射
fn resolve_dests(doc: &Document) -> Result<IndexMap<String, ObjectId>, anyhow::Error> {
    let catalog = doc.catalog()?;
    let mut raw: IndexMap<Vec<u8>, Destination> = IndexMap::new();
    let _ = doc.get_named_destinations(&catalog, &mut raw);

    let mut map: IndexMap<String, ObjectId> = IndexMap::new();
    for (name, dest) in &raw {
        if let Some(oid) = dest.page().and_then(|obj| {
            if let Object::Reference(id) = obj { Some(*id) } else { None }
        }) {
            map.insert(String::from_utf8_lossy(name).to_string(), oid);
        }
    }
    Ok(map)
}

/// 在栈中查找父级书签 ID
fn find_parent(stack: &mut Vec<(usize, u32)>, level: usize) -> Option<u32> {
    while let Some(&(l, _)) = stack.last() {
        if l < level {
            return stack.last().map(|&(_, id)| id);
        }
        stack.pop();
    }
    None
}

// ── 元数据 ──

fn add_metadata(doc: &mut Document, title: &str, authors: &[String], desc: &str, lang: &str) {
    use lopdf::Dictionary;
    let mut info = Dictionary::new();
    if !title.is_empty() { info.set("/Title", Object::string_literal(title)); }
    if !authors.is_empty() { info.set("/Author", Object::string_literal(authors.join(", ").as_str())); }
    if !desc.is_empty() { info.set("/Subject", Object::string_literal(desc)); }
    if !lang.is_empty() { info.set("/Lang", Object::string_literal(lang)); }
    info.set("/Creator", Object::string_literal("mdbook-plugins (mdbook-pdf)"));
    doc.trailer.set("Info", Object::Dictionary(info));
}

// ── 辅助 ──

fn css_escape(id: &str) -> String {
    id.chars().map(|c| match c {
        ':' | '.' | '#' | '[' | ']' | '(' | ')' | ' ' | '"' | '\'' | '!' | '@' | '$' | '%'
        | '^' | '&' | '*' | '+' | ',' | '~' | '|' | '/' | '\\' | '>' | '`' => format!("\\{}", c),
        _ => c.to_string(),
    }).collect()
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_basic() {
        let html = "<html><body>
            <a class=\"header\" href=\"#intro\"></a>
            <h1 id=\"intro\">Introduction</h1>
            <a class=\"header\" href=\"#setup\"></a>
            <h2 id=\"setup\">Setup Guide</h2>
            <a class=\"header\" href=\"#end\"></a>
            <h1 id=\"end\">Conclusion</h1>
        </body></html>";

        let es = extract_bookmark_entries(html).unwrap();
        assert_eq!(es.len(), 3);
        assert_eq!(es[0].level, 1);
        assert_eq!(es[0].title, "Introduction");
        assert_eq!(es[1].level, 2);
        assert_eq!(es[1].title, "Setup Guide");
    }

    #[test]
    fn test_find_parent() {
        let mut s: Vec<(usize, u32)> = vec![];
        assert_eq!(find_parent(&mut s, 1), None);
        s.push((1, 10));
        assert_eq!(find_parent(&mut s, 2), Some(10));
        let mut s2: Vec<(usize, u32)> = vec![(1, 10), (2, 11)];
        assert_eq!(find_parent(&mut s2, 1), None);
        assert!(s2.is_empty());
    }
}
