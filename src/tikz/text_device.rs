//! TikZ 文本层：从 PDF 页面收集字形 Unicode 与位置，生成透明 `<text>` 层。
//!
//! 视觉层由 hayro-svg（路径轮廓）负责，保证 LaTeX 精确排版且不受页面字体
//! 影响。本模块用第二遍 `interpret_page` 驱动一个轻量 [`Device`]，在每个
//! `draw_glyph` 回调中通过 [`Glyph::as_unicode`]（ToUnicode CMap / Adobe Glyph
//! List）拿到字形对应的 Unicode 文本，并用 `transform * glyph_transform` 推导
//! 基线原点与字号。收集完成后按"字号 + 基线 y"聚成行，输出 `fill="transparent"`
//! 的 `<text>` 元素，使图内文字在浏览器/PDF 中可选中、可搜索，视觉不变。

use anyhow::Result;
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image,
    InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
};
use kurbo::{Affine, BezPath, Rect};
use std::cmp::Ordering;

/// 收集到的单个字形文本片段
#[derive(Debug, Clone)]
struct TextChar {
    /// 该字形对应的 Unicode 文本（连字可能为多个字符）
    text: String,
    /// 基线原点 x（页面坐标，SVG Y 向下）
    x: f64,
    /// 基线原点 y
    y: f64,
    /// 页面字号（SVG 单位）
    font_size: f64,
    /// 前一字符水平步进的估计（px，用于"词间空格"启发）
    advance_px: f64,
    /// 变换是否轴对齐（无旋转/倾斜），可并入整行
    axis_aligned: bool,
    /// 完整变换矩阵 [a b c d e f]（非轴对齐时用于 transform 属性）
    matrix: [f64; 6],
}

/// 文本收集 Device：除 `draw_glyph` 外其余绘制操作全部忽略
pub(crate) struct TextCollector {
    chars: Vec<TextChar>,
}

impl TextCollector {
    pub(crate) fn new() -> Self {
        Self { chars: Vec::new() }
    }

    /// 收集完成：合并成行并生成 `<text>` 层 SVG 片段
    pub(crate) fn finish(self) -> String {
        if self.chars.is_empty() {
            return String::new();
        }
        let mut chars = self.chars;

        // 非轴对齐字形（旋转/倾斜文本）单独输出，每字形一个 <text transform>
        let mut standalone = String::new();
        chars.retain(|ch| {
            if ch.axis_aligned {
                true
            } else {
                standalone.push_str(&format!(
                    r#"<text {} fill="transparent" font-size="{:.2}" transform="matrix({})" x="0" y="0">{}</text>"#,
                    crate::utils::SVG_TEXT_LAYER_STYLE,
                    ch.font_size,
                    fmt_matrix(ch.matrix),
                    crate::utils::escape_xml(&ch.text)
                ));
                false
            }
        });

        // 排序（字号 → 基线 y → x），使相同行的字形相邻
        chars.sort_by(|a, b| {
            a.font_size
                .partial_cmp(&b.font_size)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.y.partial_cmp(&b.y).unwrap_or(Ordering::Equal))
                .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(Ordering::Equal))
        });
        // 去掉重复字形（如 Fill+Stroke 渲染模式下同一字形被绘制两次）
        chars.dedup_by(|a, b| {
            a.x == b.x && a.y == b.y && a.text == b.text && (a.font_size - b.font_size).abs() < 0.01
        });

        struct Line {
            fs: f64,
            y: f64,
            items: Vec<(String, f64, f64)>, // (text, x, advance_px)
        }
        let mut lines: Vec<Line> = Vec::new();
        for ch in chars {
            let fs_tol = (ch.font_size * 0.05).max(0.5);
            // 基线容差 0.25em：同一行的字形基线一致；上标/下标偏移约 0.3em 以上，
            // 会被正确分为独立行，避免 "x²" 之类被并入正文行造成索引乱序
            let y_tol = ch.font_size * 0.25;
            let fits = lines.last().map_or(false, |line| {
                (line.fs - ch.font_size).abs() <= fs_tol && (line.y - ch.y).abs() <= y_tol
            });
            if fits {
                if let Some(line) = lines.last_mut() {
                    line.items.push((ch.text, ch.x, ch.advance_px));
                }
            } else {
                lines.push(Line {
                    fs: ch.font_size,
                    y: ch.y,
                    items: vec![(ch.text, ch.x, ch.advance_px)],
                });
            }
        }

        let mut out = String::new();
        for mut line in lines {
            line.items.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
            let mut text = String::new();
            let mut xs: Vec<f64> = Vec::with_capacity(line.items.len());
            let mut prev_x = 0.0f64;
            let mut prev_adv = 0.0f64;
            for (i, (t, x, adv)) in line.items.iter().enumerate() {
                // 词间空格启发：相邻字形 x 间距明显超过前一字形步进 → 补空格，
                // 使 "Hello World" 这类文本可被按词搜索（LaTeX 词距 ≈ 0.25–0.33em）。
                // 插入空格的同时必须补一个对应的 x 坐标（空格自然位置 = 前一字符
                // 位置 + 其步进），否则空格会消耗后续字符的 x 值、末尾字符堆叠，
                // 造成文字选中/搜索高亮与视觉位置错位。
                if i > 0 && (x - prev_x) - prev_adv > line.fs * 0.25 {
                    text.push(' ');
                    xs.push(prev_x + prev_adv);
                }
                text.push_str(t);
                xs.push(*x);
                prev_x = *x;
                prev_adv = *adv;
            }
            let xs_str = xs
                .iter()
                .map(|x| format!("{:.2}", x))
                .collect::<Vec<_>>()
                .join(" ");
            out.push_str(&format!(
                r#"<text {} fill="transparent" font-size="{:.2}" y="{:.2}" x="{}">{}</text>"#,
                crate::utils::SVG_TEXT_LAYER_STYLE,
                line.fs,
                line.y,
                xs_str,
                crate::utils::escape_xml(&text)
            ));
        }
        out.push_str(&standalone);
        out
    }
}

impl<'a> Device<'a> for TextCollector {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        _paint: &Paint<'a>,
        _mode: &GlyphDrawMode,
    ) {
        let Some(bf) = glyph.as_unicode() else { return };
        let text = match bf {
            BfString::Char(c) => c.to_string(),
            BfString::String(s) => s,
        };
        if text.is_empty() {
            return;
        }

        // 字形在页面上的完整变换 = ctm × 文本矩阵 × 字形缩放
        let tf = transform * glyph_transform;
        let c = tf.as_coeffs();
        // 字号：y 方向列向量长度 × upem(1000)
        let font_size = (c[2] * c[2] + c[3] * c[3]).sqrt() * 1000.0;
        if !font_size.is_finite() || font_size <= 0.0 {
            return;
        }
        // 前一字符步进估计（均匀缩放假设下：advance/1000 × 字号）
        let advance_px = match glyph {
            Glyph::Outline(o) => o.advance_width().unwrap_or(0.0) as f64 * font_size / 1000.0,
            Glyph::Type3(_) => 0.0,
        };
        let axis_aligned = c[1].abs() < 1e-3 && c[2].abs() < 1e-3;

        self.chars.push(TextChar {
            text,
            x: c[4],
            y: c[5],
            font_size,
            advance_px,
            axis_aligned,
            matrix: c,
        });
    }

    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// 对 PDF 页面做第二遍解释，返回 `<text>` 层 SVG 片段（可能为空串）
pub(crate) fn collect_page_text_layer(
    page: &Page<'_>,
    settings: &InterpreterSettings,
) -> Result<String> {
    // 文本收集是独立解释过程，使用自己的解释器缓存（hayro-svg 的
    // RenderCache 内部字段不对外公开）
    let cache = InterpreterCache::new();
    let mut state = Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(
            0.0,
            0.0,
            page.render_dimensions().0 as f64,
            page.render_dimensions().1 as f64,
        ),
        &cache,
        page.xref(),
        settings.clone(),
    );
    let mut collector = TextCollector::new();
    interpret_page(page, &mut state, &mut collector);
    Ok(collector.finish())
}

/// 格式化 SVG matrix 属性值
fn fmt_matrix(c: [f64; 6]) -> String {
    c.iter()
        .map(|v| format!("{:.4}", v))
        .collect::<Vec<_>>()
        .join(" ")
}
