/// Minimal RFC-4180-ish CSV reader: handles quoted fields, embedded commas,
/// doubled "" escapes, and CRLF/LF line endings. Enough for a batch manifest;
/// not a general-purpose CSV library.
pub fn parse(content: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut field_started = false;

    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }

        match c {
            '"' => {
                in_quotes = true;
                field_started = true;
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                field_started = false;
            }
            '\r' => {} // handled by the \n case
            '\n' => {
                row.push(std::mem::take(&mut field));
                field_started = false;
                rows.push(std::mem::take(&mut row));
            }
            _ => {
                field.push(c);
                field_started = true;
            }
        }
    }

    // Flush any trailing field/row not terminated by a newline.
    if field_started || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn quoted_commas_and_escapes() {
        let rows = parse("name,text\r\na,\"x, \"\"y\"\"\"\r\nb,z");
        assert_eq!(
            rows,
            vec![vec!["name", "text"], vec!["a", "x, \"y\""], vec!["b", "z"]]
        );
    }

    #[test]
    fn blank_lines_yield_single_empty_field() {
        let rows = parse("a,b\n\nc,d\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec![""]);
    }
}
