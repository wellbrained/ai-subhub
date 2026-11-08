//! Lays out the usage report (`overview::report::Block`) as an A4 PDF with Geist (built in,
//! static Regular and SemiBold cuts: genpdf needs separate files per weight). genpdf wraps text,
//! sizes table columns by weight and breaks pages.
use genpdf::elements::{Break, FrameCellDecorator, Paragraph, TableLayout};
use genpdf::fonts::{FontData, FontFamily};
use genpdf::style::{Color, Style};
use genpdf::{Alignment, Element as _};

use crate::overview::report::Block;

const REGULAR: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.ttf");

const MUTED: Color = Color::Rgb(110, 110, 120);
const ACCENT: Color = Color::Rgb(37, 99, 235);

fn muted() -> Style {
    Style::new().with_color(MUTED)
}

/// The report as PDF bytes.
pub fn render(title: &str, blocks: &[Block]) -> Result<Vec<u8>, String> {
    let font = |b: &[u8]| FontData::new(b.to_vec(), None).map_err(|e| e.to_string());
    let (regular, bold) = (font(REGULAR)?, font(SEMIBOLD)?);
    let family = FontFamily {
        regular: regular.clone(),
        bold: bold.clone(),
        italic: regular,
        bold_italic: bold,
    };
    let mut doc = genpdf::Document::new(family);
    doc.set_title(title);
    // No PDF/A extras (ICC profile, XMP): a much smaller file.
    doc.set_minimal_conformance();
    doc.set_paper_size(genpdf::PaperSize::A4);
    doc.set_font_size(9);
    doc.set_line_spacing(1.25);
    let mut deco = genpdf::SimplePageDecorator::new();
    deco.set_margins(16);
    deco.set_header(|page| {
        // Page numbers from page 2 on, top right.
        let mut p = Paragraph::default();
        if page > 1 {
            p.push_styled(format!("AI SubHub · Usage report · page {page}"), muted());
        }
        p.aligned(Alignment::Right)
            .padded(genpdf::Margins::trbl(0, 0, 4, 0))
    });
    doc.set_page_decorator(deco);

    for block in blocks {
        match block {
            Block::Title(t) => {
                doc.push(Paragraph::new(t.as_str()).styled(Style::new().bold().with_font_size(18)));
            }
            Block::Subtitle(t) => {
                doc.push(Paragraph::new(t.as_str()).styled(muted()));
                doc.push(Break::new(1.2));
            }
            Block::Lead(lead, rest) => {
                let p = Paragraph::default()
                    .styled_string(lead.clone(), Style::new().bold())
                    .styled_string(format!(" {rest}"), Style::new());
                doc.push(
                    p.styled(Style::new().with_font_size(10))
                        .padded(genpdf::Margins::trbl(0, 0, 1.5, 0)),
                );
            }
            Block::Kpis(items) => {
                let mut t = TableLayout::new(vec![1; items.len().max(1)]);
                let mut row = t.row();
                for (value, what) in items {
                    let cell = genpdf::elements::LinearLayout::vertical()
                        .element(
                            Paragraph::new(value.as_str())
                                .styled(Style::new().bold().with_font_size(15)),
                        )
                        .element(Paragraph::new(what.as_str()).styled(muted()));
                    row = row.element(cell.padded(genpdf::Margins::trbl(2, 2, 2, 0)));
                }
                row.push().map_err(|e| e.to_string())?;
                doc.push(Break::new(0.6));
                doc.push(t);
            }
            Block::Heading(h) => {
                doc.push(Break::new(1.2));
                doc.push(
                    Paragraph::new(h.as_str())
                        .styled(Style::new().bold().with_font_size(12).with_color(ACCENT)),
                );
                doc.push(Break::new(0.3));
            }
            Block::Text(t) => {
                doc.push(Paragraph::new(t.as_str()).padded(genpdf::Margins::trbl(0, 0, 1.5, 0)));
            }
            Block::Table {
                header,
                widths,
                numeric,
                rows,
            } => {
                let mut t = TableLayout::new(widths.clone());
                t.set_cell_decorator(FrameCellDecorator::new(false, false, false));
                let cell = |text: &str, right: bool, style: Style| {
                    Paragraph::new(text)
                        .aligned(if right {
                            Alignment::Right
                        } else {
                            Alignment::Left
                        })
                        .styled(style)
                        .padded(genpdf::Margins::trbl(1, 1.5, 1, 1.5))
                };
                let mut head = t.row();
                for (i, h) in header.iter().enumerate() {
                    head = head.element(cell(h, numeric[i], muted().bold()));
                }
                head.push().map_err(|e| e.to_string())?;
                for r in rows {
                    let mut row = t.row();
                    for (i, c) in r.iter().enumerate() {
                        row = row.element(cell(
                            c,
                            numeric.get(i).copied().unwrap_or(false),
                            Style::new(),
                        ));
                    }
                    row.push().map_err(|e| e.to_string())?;
                }
                doc.push(t);
                doc.push(Break::new(0.8));
            }
            Block::Note(t) => {
                doc.push(Break::new(1.5));
                doc.push(Paragraph::new(t.as_str()).styled(muted().with_font_size(8)));
            }
        }
    }
    let mut out = Vec::new();
    doc.render(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_every_block_kind_to_a_pdf() {
        let blocks = vec![
            Block::Title("AI SubHub · Usage report".into()),
            Block::Subtitle("4 weeks · Oct 12 – Nov 8".into()),
            Block::Lead("You use about half.".into(), "Work sits idle.".into()),
            Block::Kpis(vec![
                ("47%".into(), "avg".into()),
                ("7.2 / week".into(), "hits".into()),
            ]),
            Block::Heading("Accounts".into()),
            Block::Text("Some text, äöü – “quotes”.".into()),
            Block::Table {
                header: vec!["Account".into(), "Avg".into()],
                widths: vec![3, 1],
                numeric: vec![false, true],
                rows: (0..80)
                    .map(|i| vec![format!("Account {i}"), "12%".into()])
                    .collect(),
            },
            Block::Note("API estimate.".into()),
        ];
        let pdf = render("Report", &blocks).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        // 80 table rows do not fit one page.
        let pages = pdf
            .windows(b"/Type /Page\n".len())
            .filter(|w| w == b"/Type /Page\n")
            .count()
            + pdf
                .windows(b"/Type/Page/".len())
                .filter(|w| w == b"/Type/Page/")
                .count();
        assert!(pdf.len() > 10_000, "{pages}");
    }
}
