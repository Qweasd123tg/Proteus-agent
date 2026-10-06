use std::collections::HashSet;

use ammonia::Builder as HtmlSanitizer;
use pulldown_cmark::{Options as MarkdownOptions, Parser, html};

mod blocks;
mod math;
pub(crate) use blocks::Block;
pub(crate) use blocks::blocks as markdown_blocks;
mod preview;
#[cfg(test)]
mod tests;
use math::{extract_math_fragments, normalize_math_code_blocks};
pub(crate) use preview::highlight_preview;

pub(crate) fn markdown_html(text: &str) -> String {
    let normalized_text = normalize_math_code_blocks(text);
    let (markdown_text, math_fragments) = extract_math_fragments(&normalized_text);
    let parser = Parser::new_ext(&markdown_text, options());
    let mut output = String::new();
    html::push_html(&mut output, parser);
    for (token, html) in math_fragments {
        output = output.replace(&token, &html);
    }
    sanitize_html(&enhance_code_blocks(&output))
}

fn options() -> MarkdownOptions {
    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_TABLES);
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    options.insert(MarkdownOptions::ENABLE_TASKLISTS);
    options
}

fn sanitize_html(html: &str) -> String {
    HtmlSanitizer::default()
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .add_tags(["button", "details", "summary"])
        .add_tag_attributes("details", ["open"])
        .add_generic_attributes(["class"])
        .add_tag_attributes("button", ["type", "title"])
        .clean(html)
        .to_string()
}

pub(crate) fn plain_text_html(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            '\n' => output.push_str("<br>"),
            _ => output.push(ch),
        }
    }
    output
}

/// Оборачивает каждый `<pre><code>` блок в контейнер с шапкой: ярлык языка,
/// кнопки copy и wrap (обработчик кликов делегирован в ui/markdown.js). Поиск по
/// литералу безопасен: pulldown-cmark экранирует `<`/`>` внутри кода, поэтому
/// `</code></pre>` не встретится в содержимом блока.
fn enhance_code_blocks(html: &str) -> String {
    const OPEN: &str = "<pre><code";
    const CLOSE: &str = "</code></pre>";
    const PRE_LEN: usize = 5; // "<pre>"

    let mut out = String::with_capacity(html.len() + 96);
    let mut rest = html;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(gt) = after[PRE_LEN..].find('>').map(|index| PRE_LEN + index + 1) else {
            out.push_str(after);
            return out;
        };
        let Some(close) = after[gt..].find(CLOSE).map(|index| gt + index) else {
            out.push_str(after);
            return out;
        };
        let lang = code_block_language(&after[..gt]);
        let block = &after[..close + CLOSE.len()];
        out.push_str(&format!(
            "<div class=\"code-block\"><div class=\"code-block-head\">\
<span class=\"code-lang\">{lang}</span>\
<span class=\"code-actions\">\
<button class=\"code-wrap\" type=\"button\" aria-pressed=\"false\">Перенос</button>\
<button class=\"code-copy\" type=\"button\">Копировать</button>\
</span></div>{block}</div>"
        ));
        rest = &after[close + CLOSE.len()..];
    }
    out.push_str(rest);
    out
}

fn code_block_language(open_tag: &str) -> String {
    if let Some(index) = open_tag.find("language-") {
        let lang = open_tag[index + "language-".len()..]
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '+' | '#'))
            .collect::<String>();
        if !lang.is_empty() {
            return lang;
        }
    }
    "code".to_owned()
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
