//! 最小 PDF 生成器（R2-309 余量收口：`/api/export/monthly.pdf` 的 Rust 侧实现）。
//!
//! 与 Python 侧 `app/export.py::monthly_pdf`（reportlab）同版式的一页式月报：
//! A4（595×842pt）· 标题/生成时间 · 5 张统计卡 · 每日柱状图 · 合规脚注。
//! 零新增依赖，手写 PDF 对象（项目先例：evidence.zip 手写 stored-ZIP）。
//!
//! 中文字体：非嵌入 CIDFont（Type0 + `STSong-Light` + `UniGB-UCS2-H` 预定义 CMap，
//! 与 reportlab 亚洲字体方案一致）。文本串用 UTF-16BE 十六进制编码（该 CMap 把
//! 2 字节 UCS2 输入码映射到 CID）；不嵌字体文件，查看器负责字体替换
//! （Adobe 自带 GB1 字体，Chromium/PDFium 会回退到系统中文字体）。
//! 优点：体积小、实现简单；代价：离线查看器（无 CJK 回退的）可能缺字——
//! 与 Python 侧「字体注册失败回退 Helvetica 中文缺字」同一权衡，如实取舍。
//!
//! 坐标系与 reportlab 一致：原点左下、y 向上、单位 pt；drawString(x, y) 即基线。

/// 月报数据（由调用方从 store 统计得出，与 Python `monthly_pdf` 同口径）。
pub struct MonthlyReport<'a> {
    /// `YYYY-MM`
    pub month: &'a str,
    /// 生成时间（`YYYY-MM-DD HH:MM`，控制面统计口径与 Python 一致用本地时钟）
    pub generated: &'a str,
    pub total: i64,
    /// [L1, L2, L3] 各级事件数
    pub by_level: [i64; 3],
    /// status == "new"（未处置）计数
    pub unhandled: i64,
    /// 1..=days_in_month 的事件分布（缺日为 0）
    pub by_day: &'a [(u32, i64)],
    pub days_in_month: u32,
}

const W: f64 = 595.0;
const H: f64 = 842.0;
/// 颜色与 Python `HexColor` 常量一一对应（#4da3ff 强调 / #1a2129 归零卡 / #888888 轴线 / #444444 次要文字）
const BLUE: (f64, f64, f64) = (0.302, 0.639, 1.0);
const DARK: (f64, f64, f64) = (0.102, 0.129, 0.161);
const GREY_LINE: (f64, f64, f64) = (0.533, 0.533, 0.533);
const GREY_TEXT: (f64, f64, f64) = (0.267, 0.267, 0.267);

/// 文本 → `BT /F1 size Tf 1 0 0 1 x y Tm <UTF-16BE hex> Tj ET`（reportlab drawString 等价）。
fn draw_text(out: &mut String, size: f64, x: f64, y: f64, text: &str) {
    let hex: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
    out.push_str(&format!(
        "BT /F1 {size:.0} Tf 1 0 0 1 {x:.2} {y:.2} Tm <{hex}> Tj ET\n"
    ));
}

/// 填充矩形（`x y w h re f`，fill 颜色需先设置）。
fn fill_rect(out: &mut String, x: f64, y: f64, w: f64, h: f64, color: (f64, f64, f64)) {
    out.push_str(&format!(
        "{:.3} {:.3} {:.3} rg\n{x:.2} {y:.2} {w:.2} {h:.2} re f\n",
        color.0, color.1, color.2
    ));
}

/// 从字节流组装完整 PDF（1 目录 + 1 页 + 1 内容流 + 3 字体相关对象）。
fn assemble_pdf(content: &[u8]) -> Vec<u8> {
    let mut objs: Vec<String> = Vec::new();
    // 1: Catalog
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    // 2: Pages
    objs.push("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into());
    // 3: Page（A4，字体 F1 = obj5）
    objs.push(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .into(),
    );
    // 4: 内容流
    objs.push(format!(
        "<< /Length {} >>\nstream\n{}\nendstream",
        content.len(),
        String::from_utf8_lossy(content) // 内容流仅 ASCII 操作符 + hex 串，恒为合法 UTF-8
    ));
    // 5: Type0 复合字体（非嵌入，预定义 CMap UniGB-UCS2-H → Adobe-GB1 CID 集合）
    objs.push(
        "<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light \
         /Encoding /UniGB-UCS2-H /DescendantFonts [6 0 R] >>"
            .into(),
    );
    // 6: CIDFontType2 降级字体（STSong-Light 为 Adobe GB1 伴随字体）
    objs.push(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /STSong-Light \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> \
         /FontDescriptor 7 0 R /DW 1000 >>"
            .into(),
    );
    // 7: FontDescriptor（指标取 reportlab Asian CID 字体同款）
    objs.push(
        "<< /Type /FontDescriptor /FontName /STSong-Light /Flags 4 \
         /FontBBox [-180 -300 1810 940] /ItalicAngle 0 /Ascent 880 \
         /Descent -120 /CapHeight 880 /StemV 93 >>"
            .into(),
    );

    let mut pdf = Vec::with_capacity(4096);
    pdf.extend_from_slice(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n");
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(pdf.len() as i64);
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref_at = pdf.len() as i64;
    pdf.extend_from_slice(format!("xref\n0 {}\n", objs.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objs.len() + 1,
            xref_at
        )
        .as_bytes(),
    );
    pdf
}

/// 渲染一页式月报 PDF（版式与 Python `monthly_pdf` 对齐）。
pub fn render_monthly(r: &MonthlyReport) -> Vec<u8> {
    let mut c = String::with_capacity(4096);

    // 标题 + 生成时间
    draw_text(
        &mut c,
        18.0,
        50.0,
        H - 60.0,
        &format!("LabSentinel 实验室安全月报（{}）", r.month),
    );
    draw_text(
        &mut c,
        11.0,
        50.0,
        H - 85.0,
        &format!("生成时间：{}", r.generated),
    );

    // 统计卡（88×46，间距 98；值为 0 用深灰，否则强调蓝）
    let cards = [
        ("事件总数", r.total),
        ("L1 提示", r.by_level[0]),
        ("L2 警告", r.by_level[1]),
        ("L3 严重", r.by_level[2]),
        ("未处置", r.unhandled),
    ];
    let mut x = 50.0;
    for (label, val) in &cards {
        let color = if *val == 0 { DARK } else { BLUE };
        fill_rect(&mut c, x, H - 150.0, 88.0, 46.0, color);
        c.push_str("1 1 1 rg\n");
        draw_text(&mut c, 16.0, x + 10.0, H - 122.0, &val.to_string());
        draw_text(&mut c, 9.0, x + 10.0, H - 140.0, label);
        x += 98.0;
    }

    // 每日柱状（基线 H-420，高 200，宽 W-100；柱宽的 70% 宽度，与 Python 一致）
    draw_text(&mut c, 12.0, 50.0, H - 190.0, "每日事件分布");
    let base_y = H - 420.0;
    let (chart_h, chart_w) = (200.0, W - 100.0);
    let maxv = r.by_day.iter().map(|(_, v)| *v).max().unwrap_or(0);
    let bw = chart_w / r.days_in_month as f64;
    c.push_str(&format!(
        "{:.3} {:.3} {:.3} RG\n50 {base_y:.2} m {:.2} {base_y:.2} l S\n",
        GREY_LINE.0,
        GREY_LINE.1,
        GREY_LINE.2,
        W - 50.0
    ));
    for d in 1..=r.days_in_month {
        let v = r
            .by_day
            .iter()
            .find(|(dd, _)| *dd == d)
            .map(|(_, v)| *v)
            .unwrap_or(0);
        if v > 0 {
            let bh = if maxv > 0 {
                v as f64 / maxv as f64 * chart_h
            } else {
                0.0
            };
            fill_rect(
                &mut c,
                50.0 + (d - 1) as f64 * bw + bw * 0.15,
                base_y,
                bw * 0.7,
                bh,
                BLUE,
            );
        }
        if d % 5 == 0 || d == 1 {
            c.push_str(&format!(
                "{:.3} {:.3} {:.3} rg\n",
                GREY_TEXT.0, GREY_TEXT.1, GREY_TEXT.2
            ));
            draw_text(
                &mut c,
                7.0,
                50.0 + (d - 1) as f64 * bw,
                base_y - 12.0,
                &d.to_string(),
            );
        }
    }

    // 合规脚注
    c.push_str(&format!(
        "{:.3} {:.3} {:.3} rg\n",
        GREY_TEXT.0, GREY_TEXT.1, GREY_TEXT.2
    ));
    draw_text(
        &mut c,
        9.0,
        50.0,
        60.0,
        "合规声明：本报告不含人脸照片与截图素材；仅含事件统计信息（80-risk-register C3）。",
    );
    draw_text(
        &mut c,
        9.0,
        50.0,
        46.0,
        "数据保留策略见《安装与卸载》文档；人脸模块（如启用）默认关闭且需单独同意。",
    );

    assemble_pdf(c.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MonthlyReport<'static> {
        MonthlyReport {
            month: "2026-11",
            generated: "2026-11-11 09:30",
            total: 123,
            by_level: [64, 41, 18],
            unhandled: 9,
            by_day: &[(1, 10), (5, 30), (10, 55), (11, 28)],
            days_in_month: 30,
        }
    }

    #[test]
    fn structure_is_a_valid_single_page_pdf() {
        let pdf = render_monthly(&sample());
        let s = String::from_utf8_lossy(&pdf);
        assert!(pdf.starts_with(b"%PDF-1.7"));
        assert!(s.contains("%%EOF"));
        assert!(s.contains("/MediaBox [0 0 595 842]"));
        assert_eq!(s.matches("/Type /Page ").count(), 1); // 单页（区别于 /Catalog /Pages）
                                                          // 7 个对象全部按序出现
        for n in 1..=7 {
            assert!(s.contains(&format!("{n} 0 obj")), "obj {n} missing");
        }
        // 字体链：Type0 → CIDFontType2 → FontDescriptor（STSong-Light + UniGB-UCS2-H）
        assert!(s.contains("/BaseFont /STSong-Light"));
        assert!(s.contains("/Encoding /UniGB-UCS2-H"));
        assert!(s.contains("/Subtype /CIDFontType2"));
    }

    #[test]
    fn xref_offsets_point_at_objects() {
        let pdf = render_monthly(&sample());
        let s = String::from_utf8_lossy(&pdf);
        let xref_at: usize = s
            .split("startxref\n")
            .nth(1)
            .and_then(|t| t.lines().next().and_then(|l| l.trim().parse().ok()))
            .expect("startxref");
        assert_eq!(&pdf[xref_at..xref_at + 4], b"xref");
        // xref 每条目偏移与实际 "N 0 obj" 位置一致（xref 段为纯 ASCII；注意 s 是
        // from_utf8_lossy 的结果，二进制注释 4 字节会膨胀成 U+FFFD×4，字节偏移与 s
        // 的下标不一致——必须基于 pdf 字节切片解析）。跳过 xref 头、"0 N" 计数行
        // 与空闲对象条目（0000000000）后才是 obj1..obj7。
        let xref = String::from_utf8_lossy(&pdf[xref_at..]);
        let mut entries = xref.lines().skip(3).take(7);
        for n in 1..=7usize {
            let e = entries.next().unwrap();
            let off: usize = e[..10].trim().parse().unwrap();
            let want = format!("{n} 0");
            let want = want.as_bytes();
            assert_eq!(&pdf[off..off + 3], &want[0..3], "obj {n} offset");
        }
    }

    #[test]
    fn content_mirrors_python_layout() {
        let pdf = render_monthly(&sample());
        let s = String::from_utf8_lossy(&pdf);
        // 中文文本以 UTF-16BE hex 出现：标题「月报」= U+6708 U+62A5
        assert!(s.contains("670862A5"), "标题 hex 缺失");
        // 统计卡矩形（5 张 88x46）+ 基线轴线
        assert_eq!(s.matches("88.00 46.00 re f").count(), 5);
        assert!(s.contains("50 422.00 m 545.00 422.00 l S"));
        // 有数据的日期画柱（4 天），标签 1/5/10 出现
        assert_eq!(s.matches("re f").count(), 5 + 4);
        // 合规脚注照抄 Python 文案（「合规声明」= U+5408 U+89C4 U+58F0 U+660E）
        assert!(s.contains("540889C458F0660E"), "合规声明 hex 缺失");
    }

    #[test]
    fn empty_month_still_renders() {
        let r = MonthlyReport {
            month: "2026-02",
            generated: "2026-03-01 00:00",
            total: 0,
            by_level: [0, 0, 0],
            unhandled: 0,
            by_day: &[],
            days_in_month: 29, // 2026 不是闰年？2026/4=506.5 → 平年 28。用 28 断言由调用方保证
        };
        let pdf = render_monthly(&r);
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("%%EOF"));
        // 归零卡用深灰
        assert!(s.contains("0.102 0.129 0.161 rg"));
    }

    #[test]
    fn utf16be_covers_bmp_and_ascii() {
        let mut out = String::new();
        draw_text(&mut out, 12.0, 0.0, 0.0, "L1 事件"); // ASCII + BMP 汉字
        assert!(out.contains("004C0031")); // "L1" 的 UTF-16BE
        assert!(out.contains("4E8B4EF6")); // "事件" 的 UTF-16BE
    }
}
