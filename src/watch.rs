//! Watch list: items, file load/save (.halshow files), value display.

use crate::hal::HalType;

#[derive(Clone, Debug)]
pub struct WatchItem {
    pub kind: HalType,
    pub name: String,
    /// data type from ptype/stype: bit, float, s32, u32
    pub dtype: String,
    /// 1 = writable, -1 = writable but linked, 0 = not writable
    pub writable: i8,
    pub value: String,
    pub error: bool,
}

impl WatchItem {
    pub fn file_id(&self) -> String {
        format!("{}+{}", self.kind.kw(), self.name)
    }
}

/// Parse a watchlist file: comment lines (#) and blank lines are
/// ignored, items are whitespace-separated "type+name" tokens.
/// Bare names keep type = None (caller guesses via ptype/stype).
pub fn parse_file(text: &str) -> Vec<(Option<HalType>, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        for tok in line.split_whitespace() {
            match tok.split_once('+') {
                Some((kind, name)) => out.push((HalType::from_kw(kind), name.to_string())),
                None => out.push((None, tok.to_string())),
            }
        }
    }
    out
}

/// Render a watchlist file. `multiline` matches halshow's
/// "Save Watch List (multiline)" format.
pub fn file_text(items: &[WatchItem], multiline: bool) -> String {
    if multiline {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut s = format!("# halshow watchlist created {secs}\n\n");
        for it in items {
            s.push_str(&it.file_id());
            s.push('\n');
        }
        s
    } else {
        items
            .iter()
            .map(|it| it.file_id())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comment_and_items() {
        let text = "# halshow watchlist created 123\n\npin+axis.0.pos sig+estop\nparam+t.p\n";
        let items = parse_file(text);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], (Some(HalType::Pin), "axis.0.pos".to_string()));
        assert_eq!(items[1], (Some(HalType::Sig), "estop".to_string()));
        assert_eq!(items[2], (Some(HalType::Param), "t.p".to_string()));
    }
}
