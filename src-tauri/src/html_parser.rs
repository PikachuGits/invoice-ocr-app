use scraper::{Html, Selector};
use serde::Serialize;
use std::collections::HashMap;

/// A parsed cell with its logical column span information.
#[derive(Debug, Clone, Serialize)]
pub struct Cell {
    pub text: String,
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
}

/// Parse all HTML tables in the input, returning a vector of tables,
/// each table being a vector of rows, each row a vector of Cells.
pub fn parse_structured_tables(html: &str) -> Vec<Vec<Vec<Cell>>> {
    if html.is_empty() || !html.to_lowercase().contains("<table") {
        return vec![];
    }

    let document = Html::parse_document(html);
    let table_sel = Selector::parse("table").unwrap();
    let tr_sel = Selector::parse("tr").unwrap();
    let td_sel = Selector::parse("td, th").unwrap();

    let mut tables = Vec::new();

    for table in document.select(&table_sel) {
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut occupied: HashMap<(usize, usize), bool> = HashMap::new();

        for (row_idx, tr) in table.select(&tr_sel).enumerate() {
            let mut cells = Vec::new();
            let mut col = 0usize;

            for td in tr.select(&td_sel) {
                // Skip positions occupied by rowspan/colspan from above
                while occupied.contains_key(&(row_idx, col)) {
                    col += 1;
                }

                let colspan = td
                    .attr("colspan")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                let rowspan = td
                    .attr("rowspan")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);

                // BeautifulSoup equivalent: `get_text(separator=" ", strip=True)`
                // strips every text fragment before joining.
                let text = td
                    .text()
                    .map(|fragment| fragment.trim())
                    .filter(|fragment| !fragment.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");

                let cell = Cell {
                    text,
                    row: row_idx,
                    col_start: col,
                    col_end: col + colspan - 1,
                };
                cells.push(cell);

                // Mark all positions covered by this cell as occupied
                for r in row_idx..row_idx + rowspan {
                    for c in col..col + colspan {
                        occupied.insert((r, c), true);
                    }
                }

                col += colspan;
            }

            if !cells.is_empty() {
                rows.push(cells);
            }
        }

        if !rows.is_empty() {
            tables.push(rows);
        }
    }

    tables
}

/// Parse tables and return a simple 2D text matrix (first table only).
/// Compatibility helper mirroring Python's `parse_html_table`; kept for
/// callers that only need plain text.
#[allow(dead_code)]
pub fn parse_html_table(html: &str) -> Vec<Vec<String>> {
    let tables = parse_structured_tables(html);
    if let Some(table) = tables.first() {
        table
            .iter()
            .map(|row| row.iter().map(|c| c.text.clone()).collect())
            .collect()
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_table() {
        let html = r#"
        <table>
            <tr><td>A</td><td>B</td></tr>
            <tr><td>C</td><td>D</td></tr>
        </table>
        "#;
        let tables = parse_structured_tables(html);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].len(), 2);
        assert_eq!(tables[0][0][0].text, "A");
        assert_eq!(tables[0][0][1].text, "B");
        assert_eq!(tables[0][1][0].text, "C");
        assert_eq!(tables[0][1][1].text, "D");
    }

    #[test]
    fn test_colspan() {
        let html = r#"
        <table>
            <tr><td>A</td><td colspan="2">B</td></tr>
            <tr><td>C</td><td>D</td><td>E</td></tr>
        </table>
        "#;
        let tables = parse_structured_tables(html);
        assert_eq!(tables[0][0][1].col_start, 1);
        assert_eq!(tables[0][0][1].col_end, 2);
    }

    #[test]
    fn test_rowspan() {
        let html = r#"
        <table>
            <tr><td rowspan="2">A</td><td>B</td></tr>
            <tr><td>C</td></tr>
        </table>
        "#;
        let tables = parse_structured_tables(html);
        assert_eq!(tables[0][0][0].text, "A");
        assert_eq!(tables[0][1][0].text, "C");
    }

    #[test]
    fn test_empty_table() {
        let html = "<p>No table here</p>";
        let tables = parse_structured_tables(html);
        assert!(tables.is_empty());
    }
}
