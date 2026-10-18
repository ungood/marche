//! Where each value of a YAML or JSON file is, by JSON Pointer, so errors can
//! point at the line in the file.

use std::collections::HashMap;
use std::ops::Range;

use saphyr_parser::{Event, Parser};

/// The byte range each pointer names in the text. A mapping member's range is
/// its key, so `/do/0/pause/wait` points at `wait:`. Other values' ranges are
/// where the value starts.
#[derive(Debug, Default)]
pub struct Spans(HashMap<String, Range<usize>>);

enum Frame {
    Sequence {
        pointer: String,
        next: usize,
    },
    Mapping {
        pointer: String,
        key: Option<(String, Range<usize>)>,
    },
}

impl Spans {
    /// Maps the text's first document. Text that doesn't parse maps as far as it goes.
    pub fn parse(text: &str) -> Self {
        let mut spans = HashMap::new();
        let mut stack: Vec<Frame> = Vec::new();
        for event in Parser::new_from_str(text) {
            let Ok((event, span)) = event else { break };
            let range = span.start.index()..span.end.index();
            match event {
                Event::SequenceEnd | Event::MappingEnd => {
                    stack.pop();
                    continue;
                }
                Event::DocumentEnd => break,
                Event::Scalar(..)
                | Event::Alias(_)
                | Event::SequenceStart(..)
                | Event::MappingStart(..) => {}
                _ => continue,
            }
            let pointer = match stack.last_mut() {
                None => {
                    spans.insert(String::new(), range.start..range.start);
                    String::new()
                }
                Some(Frame::Sequence { pointer, next }) => {
                    let item = format!("{pointer}/{next}");
                    *next += 1;
                    spans.insert(item.clone(), range.start..range.start);
                    item
                }
                Some(Frame::Mapping { pointer, key }) => match key.take() {
                    Some((name, key_range)) => {
                        let member = format!("{pointer}/{name}");
                        spans.insert(member.clone(), key_range);
                        member
                    }
                    None => {
                        let name = match &event {
                            Event::Scalar(value, ..) => escape(value),
                            _ => String::new(),
                        };
                        *key = Some((name, range));
                        // A key that's a collection gets a frame nobody looks up.
                        format!("{pointer}/")
                    }
                },
            };
            match event {
                Event::SequenceStart(..) => stack.push(Frame::Sequence { pointer, next: 0 }),
                Event::MappingStart(..) => stack.push(Frame::Mapping { pointer, key: None }),
                _ => {}
            }
        }
        Self(spans)
    }

    /// The range of the value at `pointer`, or of its nearest ancestor that
    /// the text has, such as the alias that stands for it.
    pub fn find(&self, pointer: &str) -> Range<usize> {
        let mut pointer = pointer;
        loop {
            if let Some(range) = self.0.get(pointer) {
                return range.clone();
            }
            match pointer.rsplit_once('/') {
                Some((parent, _)) => pointer = parent,
                None => return 0..0,
            }
        }
    }
}

/// A key as a JSON Pointer segment.
fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, range: Range<usize>) -> &str {
        let start = text[..range.start].rfind('\n').map_or(0, |at| at + 1);
        let end = text[range.start..]
            .find('\n')
            .map_or(text.len(), |at| range.start + at);
        &text[start..end]
    }

    #[test]
    fn finds_keys_and_items_in_yaml() {
        let text = "document:\n  name: hello\ndo:\n  - greet:\n      set:\n        name: Ada\n  - \"say/it\":\n      wait: { seconds: 1 }\n";
        let spans = Spans::parse(text);
        assert_eq!(&text[spans.find("/document/name")], "name");
        assert_eq!(line(text, spans.find("/do/0/greet/set")), "      set:");
        assert_eq!(line(text, spans.find("/do/1")), "  - \"say/it\":");
        assert_eq!(
            line(text, spans.find("/do/1/say~1it/wait/seconds")),
            "      wait: { seconds: 1 }"
        );
        assert_eq!(line(text, spans.find("/do/0/greet/missing")), "  - greet:");
        assert_eq!(spans.find(""), 0..0);
    }

    #[test]
    fn finds_keys_in_json() {
        let text = "{\n  \"document\": {\"name\": \"é\"},\n  \"do\": [\n    {\"greet\": {\"set\": {}}}\n  ]\n}\n";
        let spans = Spans::parse(text);
        assert_eq!(&text[spans.find("/document/name")], "\"name\"");
        assert_eq!(
            line(text, spans.find("/do/0/greet/set")),
            "    {\"greet\": {\"set\": {}}}"
        );
    }
}
