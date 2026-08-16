//! Strip ANSI/terminal control sequences for the human-readable Markdown
//! export (PRD §52: "human-readable without requiring termnote"). The raw
//! event log and the CSV export both keep control sequences intact
//! (PRD §22-23); this stripping is purely a Markdown rendering concern.

/// Remove ANSI CSI/OSC escape sequences and carriage returns, leaving plain
/// text. Deliberately simple: good enough to make exported logs readable,
/// not a full terminal emulator.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    // CSI: ESC [ ... final-byte(0x40-0x7E)
                    chars.next();
                    for c2 in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c2) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: ESC ] ... (BEL or ESC \)
                    chars.next();
                    while let Some(c2) = chars.next() {
                        if c2 == '\u{7}' {
                            break;
                        }
                        if c2 == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    // Other short escape sequences (e.g. ESC ( B): skip one
                    // more char if present.
                    chars.next();
                }
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_color_codes() {
        let input = "\u{1b}[32mgreen\u{1b}[0m text";
        assert_eq!(strip_ansi(input), "green text");
    }

    #[test]
    fn strips_osc_title_sequences() {
        let input = "\u{1b}]0;window title\u{7}visible";
        assert_eq!(strip_ansi(input), "visible");
    }

    #[test]
    fn drops_carriage_returns() {
        assert_eq!(strip_ansi("a\r\nb"), "a\nb");
    }
}
