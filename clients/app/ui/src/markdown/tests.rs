use super::*;

#[test]
fn math_placeholders_do_not_overlap_or_replace_user_literals() {
    let formulas = (0..15)
        .map(|index| format!("\\(x_{index}\\)"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!("PROTEUSMATH0END PROTEUSMATHX10END {formulas}");
    let html = markdown_html(&source);
    assert!(html.contains("PROTEUSMATH0END PROTEUSMATHX10END"));
    assert_eq!(html.matches("mathjax-inline").count(), 15);
    for index in 0..15 {
        assert!(html.contains(&format!("\\(x_{index}\\)")), "{html}");
    }
}

#[test]
fn markdown_html_preserves_inline_math_for_mathjax() {
    let html = markdown_html("Energy: $E = mc^2$.");

    assert!(html.contains(r#"<span class="mathjax-inline">\(E = mc^2\)</span>"#));
}

#[test]
fn markdown_html_removes_active_dangerous_links_and_images() {
    for markdown in [
        "[run](javascript:alert(1))",
        "[run](JaVaScRiPt:alert(1))",
        "[run](data:text/html,<script>alert(1)</script>)",
        "[download](ftp://example.com/payload)",
        "[local](file:///etc/passwd)",
        "![track](data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9YWxlcnQoMSk+)",
    ] {
        let html = markdown_html(markdown);
        let active_html = html.to_ascii_lowercase();
        assert!(
            !active_html.contains("href=\"javascript:")
                && !active_html.contains("href=\"data:")
                && !active_html.contains("href=\"ftp:")
                && !active_html.contains("href=\"file:")
                && !active_html.contains("src=\"javascript:")
                && !active_html.contains("src=\"data:"),
            "unsafe output: {html}"
        );
    }
}

#[test]
fn markdown_html_keeps_allowed_web_and_mail_links() {
    let html = markdown_html("[web](https://example.com/path?q=1) [mail](mailto:test@example.com)");

    assert!(html.contains("href=\"https://example.com/path?q=1\""));
    assert!(html.contains("href=\"mailto:test@example.com\""));
}

#[test]
fn plain_text_html_escapes_markup_and_preserves_newlines() {
    let html = plain_text_html("<b>one</b>\n& two");

    assert_eq!(html, "&lt;b&gt;one&lt;/b&gt;<br>&amp; two");
}

#[test]
fn highlight_preview_colors_json_keys_and_values() {
    let html = highlight_preview("{\n  \"path\": \"a.rs\",\n  \"n\": 12\n}");

    assert!(html.contains("<span class=\"tk-key\">\"path\"</span>"));
    assert!(html.contains("<span class=\"tk-str\">\"a.rs\"</span>"));
    assert!(html.contains("<span class=\"tk-num\">12</span>"));
}

#[test]
fn highlight_preview_colors_diff_lines() {
    let html = highlight_preview("@@ -1 +1 @@\n-old\n+new");

    assert!(html.contains("<span class=\"tk-hunk\">@@ -1 +1 @@</span>"));
    assert!(html.contains("<span class=\"tk-del\">-old</span>"));
    assert!(html.contains("<span class=\"tk-add\">+new</span>"));
}

#[test]
fn highlight_preview_colors_git_status_codes() {
    let html = highlight_preview("## main...origin/main\n M src/lib.rs\n?? new.rs\nA  staged.rs");

    assert!(html.contains("<span class=\"tk-git-branch\">## main...origin/main</span>"));
    assert!(html.contains("<span class=\"tk-git-untracked\">??</span>"));
    assert!(html.contains("<span class=\"tk-git-add\">A </span>"));
    assert!(html.contains("<span class=\"tk-git-mod\"> M</span>"));
}

#[test]
fn highlight_preview_leaves_markdown_heading_alone() {
    // `## ` в начале не должен превращать обычный текст в git-status.
    let html = highlight_preview("## Заголовок\nобычный текст");

    assert!(!html.contains("tk-git"));
}

#[test]
fn highlight_preview_colors_apply_patch_lines() {
    let html = highlight_preview(
        "*** Begin Patch\n*** Update File: src/lib.rs\n@@ old\n-old\n+new\n*** End Patch",
    );

    assert!(html.contains("<span class=\"tk-patch-boundary\">*** Begin Patch</span>"));
    assert!(html.contains("<span class=\"tk-patch-op\">*** Update File: </span>"));
    assert!(html.contains("<span class=\"tk-patch-path\">src/lib.rs</span>"));
    assert!(html.contains("<span class=\"tk-hunk\">@@ old</span>"));
    assert!(html.contains("<span class=\"tk-del\">-old</span>"));
    assert!(html.contains("<span class=\"tk-add\">+new</span>"));
}

#[test]
fn highlight_preview_escapes_markup_in_all_modes() {
    // JSON-режим: значение с тегами не должно протечь как разметка.
    assert!(highlight_preview("{\"x\": \"<img>\"}").contains("&lt;img&gt;"));
    // Patch-режим: путь в заголовке тоже экранируется.
    assert!(highlight_preview("*** Begin Patch\n*** Add File: <bad>").contains("&lt;bad&gt;"));
    // Generic-режим экранирует целиком.
    assert_eq!(highlight_preview("<script>"), "&lt;script&gt;");
}

#[test]
fn markdown_html_preserves_display_math_for_mathjax() {
    let html = markdown_html(r"\[\int_0^1 x^2 dx = \frac{1}{3}\]");

    assert!(
        html.contains(r#"<span class="mathjax-display">\[\int_0^1 x^2 dx = \frac{1}{3}\]</span>"#)
    );
}

#[test]
fn markdown_html_does_not_extract_math_inside_code_spans() {
    let html = markdown_html("Use `$x$` literally.");

    assert!(html.contains("<code>$x$</code>"));
    assert!(!html.contains("mathjax-inline"));
}

#[test]
fn markdown_html_renders_math_only_fenced_code_blocks() {
    let html = markdown_html("```tex\n$$a^2 + b^2 = c^2$$\n$$x = y$$\n```");

    assert!(html.contains(r#"<span class="mathjax-display">\[a^2 + b^2 = c^2\]</span>"#));
    assert!(html.contains(r#"<span class="mathjax-display">\[x = y\]</span>"#));
    assert!(!html.contains("<pre><code>"));
}

#[test]
fn markdown_html_renders_math_only_indented_code_blocks() {
    let html = markdown_html("    $$a^2 + b^2 = c^2$$\n    $$x = y$$");

    assert!(html.contains(r#"<span class="mathjax-display">\[a^2 + b^2 = c^2\]</span>"#));
    assert!(html.contains(r#"<span class="mathjax-display">\[x = y\]</span>"#));
    assert!(!html.contains("<pre><code>"));
}

#[test]
fn markdown_html_keeps_non_math_fenced_code_blocks_as_code() {
    let html = markdown_html("```rust\nlet price = \"$10\";\n```");

    assert!(html.contains("<pre><code"));
    assert!(html.contains("let price"));
    assert!(!html.contains("mathjax"));
}

#[test]
fn markdown_html_wraps_code_blocks_with_language_label_and_actions() {
    let html = markdown_html("```rust\nfn main() {}\n```");

    assert!(html.contains("class=\"code-block\""));
    assert!(html.contains("<span class=\"code-lang\">rust</span>"));
    assert!(html.contains("class=\"code-copy\""));
    assert!(html.contains("class=\"code-wrap\""));
    assert!(html.contains("<pre><code"));
}

#[test]
fn markdown_html_labels_unmarked_code_block_as_code() {
    let html = markdown_html("```\nplain text\n```");

    assert!(html.contains("<span class=\"code-lang\">code</span>"));
}

#[test]
fn markdown_html_wraps_each_of_multiple_code_blocks() {
    let html = markdown_html("```py\na = 1\n```\n\ntext\n\n```js\nlet b = 2;\n```");

    assert_eq!(html.matches("class=\"code-block\"").count(), 2);
    assert!(html.contains("<span class=\"code-lang\">py</span>"));
    assert!(html.contains("<span class=\"code-lang\">js</span>"));
}

#[test]
fn math_after_code_fences_is_not_mistaken_for_inline_code() {
    for fence in ["```", "````", "~~~"] {
        let source = format!(
            "{fence}rust\nlet price = \"$10\";\n{fence}\n\nВ строке: $E = mc^2$\n\n$$\n\\sum_{{k=1}}^{{n}} k = \\frac{{n(n+1)}}{{2}}\n$$\n\n{fence}mermaid\nflowchart LR\nA --> B\n{fence}"
        );
        let html = markdown_html(&source);
        assert_eq!(html.matches("mathjax-inline").count(), 1, "{html}");
        assert_eq!(html.matches("mathjax-display").count(), 1, "{html}");
        assert!(html.contains("language-mermaid"));
        assert!(html.contains("$10"));
    }
}

#[test]
fn disclosure_html_is_rendered_without_active_content() {
    let html = markdown_html(
        "<details open onclick=\"alert(1)\"><summary>Детали</summary>Текст<script>alert(1)</script><img src=x onerror=\"alert(1)\"></details>",
    );
    assert!(html.contains("<details open"), "{html}");
    assert!(html.contains("<summary>Детали</summary>"));
    assert!(!html.contains("onclick") && !html.contains("onerror") && !html.contains("<script"));
}

#[test]
fn standalone_formula_between_code_blocks_keeps_both_fences() {
    let html = markdown_html("```rust\nlet x = 1;\n```\n$$a=b$$\n```python\nprint(1)\n```");
    assert_eq!(html.matches("class=\"code-block\"").count(), 2, "{html}");
    assert_eq!(html.matches("mathjax-display").count(), 1, "{html}");
}
