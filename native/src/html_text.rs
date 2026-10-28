pub fn extract(html: &str) -> String {
    extract_internal(html, false)
}

pub fn extract_with_links(html: &str) -> String {
    extract_internal(html, true)
}

fn href(attributes: &str) -> Option<&str> {
    let mut remaining = attributes;
    while !remaining.trim().is_empty() {
        remaining = remaining.trim_start();
        let end = remaining.find(|ch: char| ch.is_whitespace() || ch == '=').unwrap_or(remaining.len());
        if end == 0 { return None; }
        let name = &remaining[..end];
        remaining = remaining[end..].trim_start();
        let Some(value) = remaining.strip_prefix('=') else { continue; };
        remaining = value.trim_start();
        let value = if let Some(quote @ ('"' | '\'')) = remaining.chars().next() {
            let (value, rest) = remaining[1..].split_once(quote)?;
            remaining = rest;
            value
        } else {
            let end = remaining.find(char::is_whitespace).unwrap_or(remaining.len());
            let value = &remaining[..end];
            remaining = &remaining[end..];
            value
        };
        if name.eq_ignore_ascii_case("href") { return Some(value); }
    }
    None
}

fn extract_internal(html: &str, preserve_links: bool) -> String {
    let lower = html.to_ascii_lowercase();
    let mut result = String::new();
    let mut position = 0;
    while position < html.len() {
        let rest = &html[position..];
        if rest.starts_with("<!--") {
            position += rest.find("-->").map_or(rest.len(), |end| end + 3);
            continue;
        }
        if rest.starts_with('<') {
            let mut quote = None;
            let mut end = None;
            for (index, character) in rest.char_indices().skip(1) {
                if let Some(delimiter) = quote {
                    if character == delimiter {
                        quote = None;
                    }
                } else if matches!(character, '"' | '\'') {
                    quote = Some(character);
                } else if character == '>' {
                    end = Some(index);
                    break;
                }
            }
            let Some(end) = end else { break };
            let tag = &lower[position + 1..position + end];
            let name = tag
                .trim_start()
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches('/');
            if preserve_links && name == "a" && !tag.trim_start().starts_with('/')
                && let Some(attributes) = rest[1..end].trim_start().get(1..)
                && let Some(link) = href(attributes) {
                let link = extract(link);
                if crate::calendar::safe_url(&link) {
                    result.push(' ');
                    result.push_str(&link);
                    result.push(' ');
                }
            }
            position += end + 1;
            if matches!(name, "script" | "style" | "svg" | "head")
                && !tag.trim_start().starts_with('/')
            {
                let closing = format!("</{name}");
                let mut tail = position;
                let mut found = false;
                while let Some(offset) = lower[tail..].find(&closing) {
                    let start = tail + offset;
                    let next = start + closing.len();
                    if lower
                        .as_bytes()
                        .get(next)
                        .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>')
                    {
                        position = lower[next..]
                            .find('>')
                            .map_or(html.len(), |end| next + end + 1);
                        found = true;
                        break;
                    }
                    tail = next;
                }
                if !found {
                    position = html.len();
                }
            }
            if matches!(
                name,
                "p" | "div"
                    | "section"
                    | "article"
                    | "li"
                    | "br"
                    | "tr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
            ) {
                if !result.ends_with('\n') {
                    result.push('\n');
                }
            } else if !result.ends_with(char::is_whitespace) {
                result.push(' ');
            }
            continue;
        }
        if rest.starts_with('&')
            && let Some(end) = rest.find(';').filter(|end| *end <= 12)
        {
            let entity = &rest[1..end];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|number| u32::from_str_radix(number, 16).ok())
                    .or_else(|| {
                        entity
                            .strip_prefix('#')
                            .and_then(|number| number.parse().ok())
                    })
                    .and_then(char::from_u32),
            };
            if let Some(character) = decoded {
                if !character.is_control() || character == '\n' {
                    result.push(character);
                }
                position += end + 1;
                continue;
            }
        }
        let character = rest.chars().next().expect("Nonempty HTML suffix");
        if character.is_whitespace() {
            if !result.ends_with(char::is_whitespace) {
                result.push(' ');
            }
        } else if !character.is_control() {
            result.push(character);
        }
        position += character.len_utf8();
    }
    result.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_chat_preserves_safe_original_material_links_without_scripts() {
        let html = "<p>Use <a class='material' HREF='https://example.com/deck?a=1&amp;b=2'>the deck</a> and <a href='javascript:alert(1)'>ignore</a></p><script>https://hidden.example</script>";
        let text = extract_with_links(html);
        assert!(text.contains("https://example.com/deck?a=1&b=2"));
        assert!(text.contains("the deck"));
        assert!(!text.contains("javascript:"));
        assert!(!text.contains("hidden.example"));
        assert!(!extract(html).contains("https://"));
    }

    #[test]
    fn standalone_slides_keep_semantic_text_not_bootstrap_or_styling() {
        let html = format!(
            "<html><HEAD><style>{}</style></HEAD><body><h1>Customer &amp; demo</h1><script>if (a < b) {{}} {}</script><svg><text>Decoration</text></svg><p>Next &#x10D; step</p></body></html>",
            "style".repeat(10_000),
            "bootstrap".repeat(10_000)
        );
        let text = extract(&html);
        assert!(text.contains("Customer & demo"));
        assert!(text.contains("Next \u{010d} step"));
        assert!(!text.contains("bootstrap"));
        assert!(!text.contains("Decoration"));
        assert!(!text.contains("style"));
    }

    #[test]
    fn attributes_comments_and_unclosed_scripts_are_not_read_as_documents() {
        let html = "<p title='not > text'>Hello</p><!-- secret --><script>bad</scripted>still bad";
        assert_eq!(extract(html), "Hello");
    }
}
