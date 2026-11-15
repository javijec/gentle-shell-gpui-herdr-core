//! Markdown helpers for previews: where the Mermaid diagrams are, so the
//! app can draw them in place of their source.

use std::ops::Range;

/// A fenced ` ```mermaid ` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidBlock {
    /// The whole block, fences included (line ends after the closing fence).
    pub range: Range<usize>,
    /// The diagram's source, between the fences.
    pub source: Range<usize>,
}

/// Every closed Mermaid block in `text`, in order. A fence is up to three
/// spaces of indent and three or more backticks or tildes with the info
/// string `mermaid`; it closes at the first line of the same character, at
/// least as long, with nothing after it. An unclosed block is left alone
/// (it is still being typed).
pub fn mermaid_blocks(text: &str) -> Vec<MermaidBlock> {
    let mut blocks = Vec::new();
    let mut lines = line_spans(text);
    while let Some((start, line)) = lines.next() {
        let Some((fence, count, info)) = fence(line) else {
            continue;
        };
        let closes = |l: &str| {
            fence_run(l)
                .is_some_and(|(c, n, rest)| c == fence && n >= count && rest.trim().is_empty())
        };
        let is_mermaid = info.split_whitespace().next() == Some("mermaid");
        let source_start = start + line.len();
        // Skip to this fence's end whatever its language, so a ```` ```mermaid ```` shown
        // inside a longer fence isn't taken for a diagram.
        let mut end = None;
        for (at, l) in lines.by_ref() {
            if closes(l) {
                end = Some((at, at + l.len()));
                break;
            }
        }
        if let (true, Some((source_end, block_end))) = (is_mermaid, end) {
            blocks.push(MermaidBlock {
                range: start..block_end,
                source: source_start..source_end,
            });
        }
    }
    blocks
}

/// `(offset, line)` with each line's ending included.
fn line_spans(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut at = 0;
    text.split_inclusive('\n').map(move |line| {
        let start = at;
        at += line.len();
        (start, line)
    })
}

/// An opening fence: its character, length and info string.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let (c, n, rest) = fence_run(line)?;
    // A backtick fence's info string can't contain backticks.
    (c == '~' || !rest.contains('`')).then_some((c, n, rest.trim()))
}

/// Three or more backticks or tildes after at most three spaces.
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let line = line.trim_end_matches(['\n', '\r']);
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let body = &line[indent..];
    let c = body.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = body.chars().take_while(|x| *x == c).count();
    (n >= 3).then(|| (c, n, &body[n..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(text: &str) -> Vec<&str> {
        mermaid_blocks(text)
            .into_iter()
            .map(|b| &text[b.source])
            .collect()
    }

    #[test]
    fn finds_mermaid_blocks_and_their_extent() {
        let text = "# T\n\n```mermaid\nflowchart LR\n  a --> b\n```\nafter\n";
        let blocks = mermaid_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(&text[blocks[0].source.clone()], "flowchart LR\n  a --> b\n");
        assert_eq!(
            &text[blocks[0].range.clone()],
            "```mermaid\nflowchart LR\n  a --> b\n```\n"
        );
    }

    #[test]
    fn other_languages_tildes_and_longer_fences() {
        let text = "```rust\nfn main() {}\n```\n~~~mermaid\nerDiagram\n~~~\n````mermaid\ngraph TD\n```\nstill inside\n````\n";
        assert_eq!(
            sources(text),
            vec!["erDiagram\n", "graph TD\n```\nstill inside\n"]
        );
    }

    #[test]
    fn a_fence_shown_inside_another_is_not_a_diagram() {
        let text = "````markdown\n```mermaid\nflowchart LR\n```\n````\n";
        assert!(sources(text).is_empty());
    }

    #[test]
    fn unclosed_and_indented_blocks_are_left_alone() {
        assert!(sources("```mermaid\nflowchart LR\n  a --> b\n").is_empty());
        // Four spaces is an indented code block, not a fence.
        assert!(sources("    ```mermaid\n    flowchart LR\n    ```\n").is_empty());
        // A closing fence with text after it doesn't close.
        assert_eq!(
            sources("```mermaid\ngraph\n``` x\n```\n"),
            vec!["graph\n``` x\n"]
        );
    }

    #[test]
    fn windows_line_endings_and_no_final_newline() {
        let text = "```mermaid\r\nflowchart LR\r\n```";
        let blocks = mermaid_blocks(text);
        assert_eq!(&text[blocks[0].source.clone()], "flowchart LR\r\n");
        assert_eq!(blocks[0].range.end, text.len());
    }
}
