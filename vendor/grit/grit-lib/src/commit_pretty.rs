//! Human-oriented commit one-line formats shared by porcelain commands.

use crate::objects::ObjectId;

/// Abbreviate `oid` to at most `abbrev_len` hex characters (minimum 4, maximum 40).
///
/// # Parameters
///
/// - `oid` — full commit object id.
/// - `abbrev_len` — desired abbreviation length (clamped to 4..=40 and to the hex length).
#[must_use]
pub fn abbrev_hex(oid: &ObjectId, abbrev_len: usize) -> String {
    let hex = oid.to_hex();
    let n = abbrev_len.clamp(4, 40).min(hex.len());
    hex[..n].to_owned()
}

/// Return the pretty subject for a commit or tag message.
///
/// The subject is the first non-empty paragraph with embedded line breaks
/// collapsed to spaces. Both LF and CRLF line endings are recognized.
///
/// # Parameters
///
/// - `message` — raw commit or tag message text.
#[must_use]
pub fn message_subject(message: &str) -> String {
    let mut subject_lines = Vec::new();
    for line in MessageLines::new(message) {
        if line.text.is_empty() {
            if !subject_lines.is_empty() {
                break;
            }
            continue;
        }
        subject_lines.push(line.text);
    }
    subject_lines.join(" ")
}

/// Return the body slice after the first message paragraph.
///
/// Leading blank lines before the first paragraph are ignored. The returned
/// body starts after the blank-line separator and any additional blank lines,
/// preserving the original body line endings and trailing newline bytes.
///
/// # Parameters
///
/// - `message` — raw commit or tag message text.
#[must_use]
pub fn message_body(message: &str) -> &str {
    let mut saw_subject = false;
    let mut body_start = message.len();
    let mut iter = MessageLines::new(message).peekable();

    while let Some(line) = iter.next() {
        if line.text.is_empty() {
            if saw_subject {
                body_start = line.next_start;
                while let Some(next) = iter.peek() {
                    if !next.text.is_empty() {
                        break;
                    }
                    body_start = next.next_start;
                    iter.next();
                }
                break;
            }
            continue;
        }
        saw_subject = true;
    }

    &message[body_start..]
}

#[derive(Clone, Copy)]
struct MessageLine<'a> {
    text: &'a str,
    next_start: usize,
}

struct MessageLines<'a> {
    message: &'a str,
    pos: usize,
}

impl<'a> MessageLines<'a> {
    fn new(message: &'a str) -> Self {
        Self { message, pos: 0 }
    }
}

impl<'a> Iterator for MessageLines<'a> {
    type Item = MessageLine<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.message.len() {
            return None;
        }
        let start = self.pos;
        let tail = &self.message[start..];
        let newline_rel = tail.find('\n');
        let (mut end, next_start) = match newline_rel {
            Some(rel) => (start + rel, start + rel + 1),
            None => (self.message.len(), self.message.len()),
        };
        if self.message.as_bytes().get(end.wrapping_sub(1)) == Some(&b'\r') && end > start {
            end -= 1;
        }
        self.pos = next_start;
        Some(MessageLine {
            text: &self.message[start..end],
            next_start,
        })
    }
}

fn parse_tz_offset_seconds(offset: &str) -> i64 {
    if offset.len() < 5 {
        return 0;
    }
    let sign = if offset.starts_with('-') { -1i64 } else { 1i64 };
    let hours: i64 = offset[1..3].parse().unwrap_or(0);
    let minutes: i64 = offset[3..5].parse().unwrap_or(0);
    sign * (hours * 3600 + minutes * 60)
}

/// Format the author/committer date as `YYYY-MM-DD` in the commit's local timezone.
///
/// Matches Git's `DATE_SHORT` mode used by `--pretty=reference` (e.g. `2005-04-07`).
#[must_use]
pub fn format_short_date_from_ident(ident: &str) -> String {
    let parts: Vec<&str> = ident.rsplitn(3, ' ').collect();
    if parts.len() < 2 {
        return ident.to_owned();
    }
    let ts_str = parts[1];
    let offset_str = parts[0];
    let Ok(ts) = ts_str.parse::<i64>() else {
        return ident.to_owned();
    };
    let offset_secs = parse_tz_offset_seconds(offset_str);
    let Ok(dt) = time::OffsetDateTime::from_unix_timestamp(ts + offset_secs) else {
        return ident.to_owned();
    };
    let format = time::format_description::parse_borrowed::<1>("[year]-[month]-[day]");
    let Ok(fmt) = format else {
        return ident.to_owned();
    };
    dt.format(&fmt).unwrap_or_else(|_| ident.to_owned())
}

/// One-line `reference` format: `abbrev (subject, YYYY-MM-DD)`.
///
/// Matches upstream `git show -s --pretty=reference` / sequencer `refer_to_commit` output.
///
/// # Parameters
///
/// - `subject_first_line` — first line of the commit message (no trailing newline).
/// - `committer_ident` — raw `committer` header line (`Name <email> epoch tz`).
/// - `abbrev_len` — abbreviation length for the hash (typically 7).
#[must_use]
pub fn format_reference_line(
    oid: &ObjectId,
    subject_first_line: &str,
    committer_ident: &str,
    abbrev_len: usize,
) -> String {
    let abbrev = abbrev_hex(oid, abbrev_len);
    let date = format_short_date_from_ident(committer_ident);
    format!("{abbrev} ({subject_first_line}, {date})")
}

/// Word-wrap `text` to `width` columns with the same wrapping behavior as Git's
/// `strbuf_add_wrapped_text`, used by the `%w(width,indent1,indent2)`
/// pretty directive.
///
/// `indent1` is the indent for the first output line, `indent2` for the rest. A
/// negative `indent1` means that `-indent1` columns have already been consumed on
/// the first line (no extra indent emitted there). With `width <= 0` the text is
/// only indented, not wrapped. Column widths are measured with display width
/// (East-Asian wide characters count as 2).
// The `unwrap()`s below rest on loop invariants: `c`/`space` are `Some` whenever
// the wrapping branch that reads them runs, and `text_pos` always sits on a
// char boundary inside `text`.
#[allow(clippy::unwrap_used)]
#[must_use]
pub fn add_wrapped_text(text: &str, indent1: i64, indent2: i64, width: i64) -> String {
    use unicode_width::UnicodeWidthChar;

    if width <= 0 {
        return add_indented_text(text, indent1, indent2);
    }

    let bytes = text.as_bytes();
    let mut out = String::new();

    // Mirror Git's `strbuf_add_wrapped_text` (utf8.c). `bol` is the start of the current line's
    // pending text; `space` (when set) is the index of the last whitespace breakpoint. `w` is the
    // current column. The `new_line` label is emulated by `do_new_line`.
    let mut bol: usize = 0;
    let mut space: Option<usize> = None;
    let mut indent: i64 = indent1;
    let mut w: i64 = indent1;
    if indent1 < 0 {
        w = -indent1;
        space = Some(0);
    }

    let mut text_pos: usize = 0;
    loop {
        let c = bytes.get(text_pos).copied();
        let is_space = is_space_byte(c);
        if c.is_none() || is_space {
            let mut do_new_line = false;
            if w <= width || space.is_none() {
                let start = if let Some(sp) = space {
                    sp
                } else {
                    if c.is_none() && text_pos == bol {
                        return out;
                    }
                    for _ in 0..indent.max(0) {
                        out.push(' ');
                    }
                    bol
                };
                out.push_str(&text[start..text_pos]);
                if c.is_none() {
                    return out;
                }
                let cc = c.unwrap();
                let mut new_space = text_pos;
                if cc == b'\t' {
                    w |= 0x07;
                } else if cc == b'\n' {
                    new_space += 1;
                    match bytes.get(new_space) {
                        Some(b'\n') => {
                            out.push('\n');
                            space = Some(new_space);
                            do_new_line = true;
                        }
                        nxt if !nxt.map(u8::is_ascii_alphanumeric).unwrap_or(false) => {
                            space = Some(new_space);
                            do_new_line = true;
                        }
                        _ => {
                            out.push(' ');
                        }
                    }
                }
                if !do_new_line {
                    space = Some(new_space);
                    w += 1;
                    text_pos += 1;
                }
            } else {
                do_new_line = true;
            }
            if do_new_line {
                out.push('\n');
                let sp = space.unwrap();
                text_pos = sp + usize::from(is_space_byte(bytes.get(sp).copied()));
                bol = text_pos;
                space = None;
                w = indent2;
                indent = indent2;
            }
            continue;
        }
        // Non-space character: advance by one display glyph.
        let ch = text[text_pos..].chars().next().unwrap();
        let gw = UnicodeWidthChar::width(ch).unwrap_or(0) as i64;
        w += gw;
        text_pos += ch.len_utf8();
    }
}

fn is_space_byte(b: Option<u8>) -> bool {
    matches!(
        b,
        Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(11) | Some(12)
    )
}

/// Indent each line of `text` (`Git strbuf_add_indented_text`): the first line by
/// `indent1` spaces and subsequent lines by `indent2`. Used when wrap width is 0.
#[must_use]
pub fn add_indented_text(text: &str, indent1: i64, indent2: i64) -> String {
    let indent1 = indent1.max(0);
    let mut out = String::new();
    let mut indent = indent1;
    let bytes = text.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        let eol = match bytes[pos..].iter().position(|&b| b == b'\n') {
            Some(i) => pos + i + 1,
            None => bytes.len(),
        };
        for _ in 0..indent {
            out.push(' ');
        }
        out.push_str(&text[pos..eol]);
        pos = eol;
        indent = indent2;
    }
    out
}

#[cfg(test)]
mod wrap_tests {
    use super::add_wrapped_text;

    #[test]
    fn wrap_width_one_decoration_with_leading_newline() {
        // t4205 "magical wrapping": the buffer after `%w(1)%+d` is "\n (tag: describe-me)%+w(2)"
        // and Git rewraps it at width 1 to "\n(tag:\ndescribe-me)%+w(2)".
        let input = "\n (tag: describe-me)%+w(2)";
        assert_eq!(
            add_wrapped_text(input, 0, 0, 1),
            "\n(tag:\ndescribe-me)%+w(2)"
        );
    }

    #[test]
    fn wrap_zero_width_is_indent_only() {
        assert_eq!(add_wrapped_text("a\nb", 2, 1, 0), "  a\n b");
    }

    #[test]
    fn wrap_simple_words() {
        // Two short words, width large enough to keep them on one line.
        assert_eq!(add_wrapped_text("foo bar", 0, 0, 80), "foo bar");
    }
}
