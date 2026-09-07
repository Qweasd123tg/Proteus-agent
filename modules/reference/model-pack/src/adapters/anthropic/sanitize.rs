pub(super) fn sanitize_provider_text(text: &str) -> String {
    strip_dsml_tags(&strip_dsml_tool_blocks(text))
        .trim()
        .to_owned()
}

#[derive(Default)]
pub(super) struct DsmlStreamFilter {
    pending: String,
    in_tool_block: bool,
}

impl DsmlStreamFilter {
    pub(super) fn filter(&mut self, text: &str) -> String {
        let mut input = String::new();
        input.push_str(&self.pending);
        input.push_str(text);
        self.pending.clear();

        let (out, pending) = self.process(&input);
        self.pending = pending;
        out
    }

    pub(super) fn finish(&mut self) -> String {
        self.in_tool_block = false;
        let pending = std::mem::take(&mut self.pending);
        if pending.starts_with("<｜") || pending.starts_with("</｜") {
            String::new()
        } else {
            pending
        }
    }

    fn process(&mut self, mut rest: &str) -> (String, String) {
        const OPEN_BLOCK: &str = "<｜｜DSML｜｜tool_calls>";
        const CLOSE_BLOCK: &str = "</｜｜DSML｜｜tool_calls>";
        const OPEN_TAG: &str = "<｜｜DSML｜｜";
        const CLOSE_TAG: &str = "</｜｜DSML｜｜";
        const MARKERS: &[&str] = &[OPEN_BLOCK, CLOSE_BLOCK, OPEN_TAG, CLOSE_TAG];

        let mut out = String::new();
        loop {
            if self.in_tool_block {
                if let Some(end) = rest.find(CLOSE_BLOCK) {
                    rest = &rest[end + CLOSE_BLOCK.len()..];
                    self.in_tool_block = false;
                    continue;
                }
                let keep = longest_marker_suffix_len(rest, &[CLOSE_BLOCK]);
                let emit_len = rest.len().saturating_sub(keep);
                return (out, rest[emit_len..].to_owned());
            }

            let next = [
                rest.find(OPEN_BLOCK).map(|idx| (idx, OPEN_BLOCK, true)),
                rest.find(OPEN_TAG).map(|idx| (idx, OPEN_TAG, false)),
                rest.find(CLOSE_TAG).map(|idx| (idx, CLOSE_TAG, false)),
            ]
            .into_iter()
            .flatten()
            .min_by_key(|(idx, _, _)| *idx);

            let Some((start, marker, is_tool_block)) = next else {
                let keep = longest_marker_suffix_len(rest, MARKERS);
                let emit_len = rest.len().saturating_sub(keep);
                out.push_str(&rest[..emit_len]);
                return (out, rest[emit_len..].to_owned());
            };

            out.push_str(&rest[..start]);
            let after_marker = &rest[start + marker.len()..];
            if is_tool_block {
                rest = after_marker;
                self.in_tool_block = true;
                continue;
            }

            let Some(end) = after_marker.find('>') else {
                return (out, rest[start..].to_owned());
            };
            rest = &after_marker[end + 1..];
        }
    }
}

fn longest_marker_suffix_len(text: &str, markers: &[&str]) -> usize {
    let mut longest = 0;
    for marker in markers {
        for end in marker
            .char_indices()
            .map(|(idx, _)| idx)
            .chain(std::iter::once(marker.len()))
            .skip(1)
        {
            if end < marker.len() && text.ends_with(&marker[..end]) {
                longest = longest.max(end);
            }
        }
    }
    longest
}

fn strip_dsml_tool_blocks(text: &str) -> String {
    const OPEN: &str = "<｜｜DSML｜｜tool_calls>";
    const CLOSE: &str = "</｜｜DSML｜｜tool_calls>";

    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + OPEN.len()..];
        if let Some(end) = after_open.find(CLOSE) {
            rest = &after_open[end + CLOSE.len()..];
        } else {
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn strip_dsml_tags(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    loop {
        let next_open = rest
            .find("<｜｜DSML｜｜")
            .map(|idx| (idx, "<｜｜DSML｜｜"))
            .into_iter()
            .chain(
                rest.find("</｜｜DSML｜｜")
                    .map(|idx| (idx, "</｜｜DSML｜｜")),
            )
            .min_by_key(|(idx, _)| *idx);
        let Some((start, marker)) = next_open else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        let after_marker = &rest[start + marker.len()..];
        let Some(end) = after_marker.find('>') else {
            return out;
        };
        rest = &after_marker[end + 1..];
    }
}
