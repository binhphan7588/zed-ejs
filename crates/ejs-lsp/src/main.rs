use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};

use serde_json::{json, Value};

fn main() -> io::Result<()> {
    Server::default().run()
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, String>,
    shutdown_requested: bool,
}

impl Server {
    fn run(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        let mut stdout = io::stdout();

        while let Some(message) = read_message(&mut reader)? {
            let value: Value = match serde_json::from_str(&message) {
                Ok(value) => value,
                Err(_) => continue,
            };

            if let Some(method) = value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                if value.get("id").is_some() {
                    self.handle_request(value, &method, &mut stdout)?;
                } else {
                    self.handle_notification(value, &method);
                }
            }
        }

        Ok(())
    }

    fn handle_request(
        &mut self,
        request: Value,
        method: &str,
        stdout: &mut impl Write,
    ) -> io::Result<()> {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => json!({
                "capabilities": {
                    "textDocumentSync": 1,
                    "documentFormattingProvider": true
                },
                "serverInfo": {
                    "name": "ejs-lsp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
            "shutdown" => {
                self.shutdown_requested = true;
                Value::Null
            }
            "textDocument/formatting" => {
                let uri = request
                    .pointer("/params/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(text) = self.documents.get(uri) else {
                    return write_response(
                        stdout,
                        json!({ "jsonrpc": "2.0", "id": id, "result": [] }),
                    );
                };
                let options = ejs_formatter::FormatOptions::from_lsp_options(
                    request.pointer("/params/options"),
                );
                json!([{
                    "range": full_document_range(text),
                    "newText": ejs_formatter::format_document(text, options)
                }])
            }
            _ => {
                return write_response(
                    stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("method not found: {method}")
                        }
                    }),
                );
            }
        };

        write_response(
            stdout,
            json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        )
    }

    fn handle_notification(&mut self, notification: Value, method: &str) {
        match method {
            "textDocument/didOpen" => {
                if let (Some(uri), Some(text)) = (
                    notification
                        .pointer("/params/textDocument/uri")
                        .and_then(Value::as_str),
                    notification
                        .pointer("/params/textDocument/text")
                        .and_then(Value::as_str),
                ) {
                    self.documents.insert(uri.to_string(), text.to_string());
                }
            }
            "textDocument/didChange" => {
                let Some(uri) = notification
                    .pointer("/params/textDocument/uri")
                    .and_then(Value::as_str)
                else {
                    return;
                };
                let Some(changes) = notification
                    .pointer("/params/contentChanges")
                    .and_then(Value::as_array)
                else {
                    return;
                };
                if let Some(text) = changes
                    .last()
                    .and_then(|change| change.get("text"))
                    .and_then(Value::as_str)
                {
                    self.documents.insert(uri.to_string(), text.to_string());
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = notification
                    .pointer("/params/textDocument/uri")
                    .and_then(Value::as_str)
                {
                    self.documents.remove(uri);
                }
            }
            "exit" if self.shutdown_requested => std::process::exit(0),
            "exit" => std::process::exit(1),
            _ => {}
        }
    }
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut content_length = None;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }

    let Some(content_length) = content_length else {
        return Ok(None);
    };
    let mut buffer = vec![0; content_length];
    reader.read_exact(&mut buffer)?;
    Ok(Some(String::from_utf8_lossy(&buffer).to_string()))
}

fn write_response(stdout: &mut impl Write, response: Value) -> io::Result<()> {
    let body = response.to_string();
    write!(stdout, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    stdout.flush()
}

fn full_document_range(text: &str) -> Value {
    let mut line = 0usize;
    let mut character = 0usize;
    for current in text.split_inclusive('\n') {
        if current.ends_with('\n') {
            line += 1;
            character = 0;
        } else {
            character = current.chars().count();
        }
    }

    json!({
        "start": { "line": 0, "character": 0 },
        "end": { "line": line, "character": character }
    })
}

mod ejs_formatter {
    use serde_json::Value;

    const DEFAULT_TAB_SIZE: usize = 2;

    #[derive(Debug, Clone, Copy)]
    pub struct FormatOptions {
        tab_size: usize,
        insert_spaces: bool,
    }

    impl Default for FormatOptions {
        fn default() -> Self {
            Self {
                tab_size: DEFAULT_TAB_SIZE,
                insert_spaces: true,
            }
        }
    }

    impl FormatOptions {
        pub fn from_lsp_options(options: Option<&Value>) -> Self {
            let mut format_options = Self::default();
            if let Some(tab_size) = options
                .and_then(|options| options.get("tabSize"))
                .and_then(Value::as_u64)
                .and_then(|tab_size| usize::try_from(tab_size).ok())
            {
                format_options.tab_size = tab_size.clamp(1, 16);
            }
            if let Some(insert_spaces) = options
                .and_then(|options| options.get("insertSpaces"))
                .and_then(Value::as_bool)
            {
                format_options.insert_spaces = insert_spaces;
            }
            format_options
        }

        fn indent_unit(self) -> String {
            if self.insert_spaces {
                " ".repeat(self.tab_size)
            } else {
                "\t".to_string()
            }
        }
    }

    #[derive(Debug, Clone)]
    enum Token {
        HtmlTag(String),
        InlineHtml(String),
        EjsTag(String),
        Text(String),
    }

    pub fn format_document(source: &str, options: FormatOptions) -> String {
        let (front_matter, body) = split_front_matter(source);
        let formatted_body = format_body(body, options);

        match front_matter {
            Some(front_matter) if formatted_body.is_empty() => front_matter.to_string(),
            Some(front_matter) => format!("{front_matter}{formatted_body}"),
            None => formatted_body,
        }
    }

    fn format_body(source: &str, options: FormatOptions) -> String {
        let mut lines = Vec::new();
        let mut indent = 0usize;

        for token in tokenize(source) {
            match token {
                Token::Text(text) => {
                    let text = collapse_ws(text.trim());
                    if !text.is_empty() {
                        push_line(&mut lines, indent, &text, options);
                    }
                }
                Token::InlineHtml(tag) => push_inline_html(&mut lines, indent, &tag, options),
                Token::EjsTag(tag) => {
                    let formatted = format_ejs_tag(&tag, options);
                    let (dedent_before, indent_after) = ejs_indent_delta(&formatted);
                    if dedent_before {
                        indent = indent.saturating_sub(1);
                    }
                    push_formatted_block(&mut lines, indent, &formatted, options);
                    if indent_after {
                        indent += 1;
                    }
                }
                Token::HtmlTag(tag) => {
                    if is_closing_tag(&tag) {
                        indent = indent.saturating_sub(1);
                    }
                    push_html_tag(&mut lines, indent, &tag, options);
                    if is_opening_tag(&tag) {
                        indent += 1;
                    }
                }
            }
        }

        let mut output = lines.join("\n");
        if source.ends_with('\n') {
            output.push('\n');
        }
        output
    }

    fn split_front_matter(source: &str) -> (Option<&str>, &str) {
        if !source.starts_with("---\n") && !source.starts_with("---\r\n") {
            return (None, source);
        }

        let marker_len = if source.starts_with("---\r\n") { 5 } else { 4 };
        let rest = &source[marker_len..];
        for marker in ["\n---\r\n", "\n---\n", "\r\n---\r\n", "\r\n---\n"] {
            if let Some(index) = rest.find(marker) {
                let end = marker_len + index + marker.len();
                return (Some(&source[..end]), &source[end..]);
            }
        }

        (None, source)
    }

    fn tokenize(source: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut cursor = 0;

        while cursor < source.len() {
            let rest = &source[cursor..];
            if rest.starts_with("<%") {
                if let Some(end) = rest.find("%>") {
                    let end = cursor + end + 2;
                    tokens.push(Token::EjsTag(source[cursor..end].to_string()));
                    cursor = end;
                } else {
                    tokens.push(Token::Text(rest.to_string()));
                    break;
                }
            } else if rest.starts_with('<') {
                if let Some(relative_end) = find_html_tag_end(rest) {
                    let end = cursor + relative_end + 1;
                    let opening = source[cursor..end].trim();
                    if let Some(inline_end) = inline_element_end(source, end, opening) {
                        tokens.push(Token::InlineHtml(
                            source[cursor..inline_end].trim().to_string(),
                        ));
                        cursor = inline_end;
                    } else {
                        tokens.push(Token::HtmlTag(opening.to_string()));
                        cursor = end;
                    }
                } else {
                    tokens.push(Token::Text(rest.to_string()));
                    break;
                }
            } else {
                let next = rest
                    .find('<')
                    .map(|index| cursor + index)
                    .unwrap_or(source.len());
                tokens.push(Token::Text(source[cursor..next].to_string()));
                cursor = next;
            }
        }

        tokens
    }

    fn find_html_tag_end(text: &str) -> Option<usize> {
        let mut quote = None;
        let mut index = 0;
        let bytes = text.as_bytes();

        while index < bytes.len() {
            let current = bytes[index] as char;

            if quote.is_none() && text[index..].starts_with("<%") {
                if let Some(end) = text[index + 2..].find("%>") {
                    index += end + 4;
                    continue;
                }
            }

            match current {
                '"' | '\'' if quote == Some(current) => quote = None,
                '"' | '\'' if quote.is_none() => quote = Some(current),
                '>' if quote.is_none() => return Some(index),
                _ => {}
            }
            index += current.len_utf8();
        }

        None
    }

    fn inline_element_end(source: &str, opening_end: usize, opening: &str) -> Option<usize> {
        if !is_opening_tag(opening) {
            return None;
        }
        let name = tag_name(opening)?;
        let closing = format!("</{name}>");
        let rest = &source[opening_end..];
        if rest.starts_with(&closing) {
            return Some(opening_end + closing.len());
        }

        if is_raw_text_tag(&name) {
            let closing_start = rest.to_ascii_lowercase().find(&closing)?;
            return Some(opening_end + closing_start + closing.len());
        }

        if let Some(closing_start) = find_matching_closing_tag(rest, &name) {
            let inner = &rest[..closing_start];
            if !inner.contains('<') && !inner.trim().is_empty() && !inner.contains('\n') {
                return Some(opening_end + closing_start + closing.len());
            }
            if is_phrasing_container(&name) && is_phrasing_content(inner) {
                return Some(opening_end + closing_start + closing.len());
            }
        }

        if !matches!(name.as_str(), "title") {
            return None;
        }

        let ejs_end = rest.strip_prefix("<%")?.find("%>")? + 4;
        if rest[ejs_end..].starts_with(&closing) {
            Some(opening_end + ejs_end + closing.len())
        } else {
            None
        }
    }

    fn find_matching_closing_tag(source: &str, name: &str) -> Option<usize> {
        let mut cursor = 0usize;
        let mut depth = 0usize;

        while cursor < source.len() {
            let rest = &source[cursor..];
            if rest.starts_with("<%") {
                let Some(end) = rest.find("%>") else {
                    return None;
                };
                cursor += end + 2;
                continue;
            }

            let Some(relative_start) = rest.find('<') else {
                return None;
            };
            cursor += relative_start;
            let rest = &source[cursor..];
            let Some(relative_end) = find_html_tag_end(rest) else {
                return None;
            };
            let tag = rest[..=relative_end].trim();
            let Some(tag_name) = tag_name(tag) else {
                cursor += relative_end + 1;
                continue;
            };

            if tag_name == name {
                if is_closing_tag(tag) {
                    if depth == 0 {
                        return Some(cursor);
                    }
                    depth = depth.saturating_sub(1);
                } else if is_opening_tag(tag) {
                    depth += 1;
                }
            }

            cursor += relative_end + 1;
        }

        None
    }

    fn format_ejs_tag(tag: &str, options: FormatOptions) -> String {
        if tag.starts_with("<%#") {
            return tag.trim().to_string();
        }

        let (open, close, code) = split_ejs_tag(tag);
        let code = format_js_code(code, options);
        if code.is_empty() {
            format!("{open}{close}")
        } else if code.contains('\n') {
            format_multiline_ejs_tag(open, close, &code)
        } else {
            format!("{open} {code} {close}")
        }
    }

    fn format_multiline_ejs_tag(open: &str, close: &str, code: &str) -> String {
        let mut lines: Vec<String> = code.lines().map(str::to_string).collect();
        let Some(first_non_empty) = lines.iter().position(|line| !line.trim().is_empty()) else {
            return format!("{open}{close}");
        };
        let Some(last_non_empty) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
            return format!("{open}{close}");
        };

        lines[first_non_empty] = format!("{open} {}", lines[first_non_empty].trim_start());
        lines[last_non_empty] = format!("{} {close}", lines[last_non_empty].trim_end());
        lines.join("\n")
    }

    fn split_ejs_tag(tag: &str) -> (&str, &str, &str) {
        let open_len = if tag.starts_with("<%-") || tag.starts_with("<%=") {
            3
        } else {
            2
        };
        let close_len = if tag.ends_with("-%>") || tag.ends_with("_%>") {
            3
        } else {
            2
        };
        (
            &tag[..open_len],
            &tag[tag.len() - close_len..],
            &tag[open_len..tag.len() - close_len],
        )
    }

    fn ejs_indent_delta(tag: &str) -> (bool, bool) {
        if tag.starts_with("<%=") || tag.starts_with("<%-") || tag.starts_with("<%#") {
            return (false, false);
        }

        let (_, _, code) = split_ejs_tag(tag);
        let code = code.trim();
        let dedent_before = code.starts_with('}');
        let indent_after = code.ends_with('{');
        (dedent_before, indent_after)
    }

    fn format_js_code(code: &str, options: FormatOptions) -> String {
        let trimmed = code.trim();
        if trimmed.contains('\n') {
            return format_multiline_js(trimmed, options);
        }
        format_single_line_js(trimmed)
    }

    fn trim_multiline_js(code: &str) -> Vec<String> {
        let lines: Vec<&str> = code.lines().collect();
        let min_indent = lines
            .iter()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(line.len() - line.trim_start().len())
                }
            })
            .min()
            .unwrap_or(0);

        lines
            .iter()
            .map(|line| {
                if line.trim().is_empty() {
                    String::new()
                } else {
                    line[min_indent..].trim_end().to_string()
                }
            })
            .collect()
    }

    fn format_multiline_js(code: &str, options: FormatOptions) -> String {
        let mut output = Vec::new();
        let mut indent = 0usize;

        for line in trim_multiline_js(code) {
            let formatted = format_single_line_js(line.trim());
            if formatted.is_empty() {
                output.push(String::new());
                continue;
            }

            let leading_closers = leading_js_closing_delimiters(&formatted);
            if leading_closers > 0 {
                indent = indent.saturating_sub(1);
            }
            output.push(format!(
                "{}{}",
                options.indent_unit().repeat(indent),
                formatted
            ));
            let opens = count_js_block_openers(&formatted);
            let closes = count_js_block_closers(&formatted);
            indent = indent
                .saturating_add(opens)
                .saturating_sub(closes.saturating_sub(leading_closers));
        }

        output.join("\n")
    }

    fn format_single_line_js(code: &str) -> String {
        let mut formatter = JsLineFormatter::new(code);
        formatter.format()
    }

    fn leading_js_closing_delimiters(line: &str) -> usize {
        line.chars()
            .take_while(|character| matches!(character, '}' | ']' | ')'))
            .count()
    }

    fn count_js_block_openers(line: &str) -> usize {
        count_js_delimiters(line, ['{', '[', '('])
    }

    fn count_js_block_closers(line: &str) -> usize {
        count_js_delimiters(line, ['}', ']', ')'])
    }

    fn count_js_delimiters(line: &str, delimiters: [char; 3]) -> usize {
        let mut count = 0usize;
        let mut state = JsScanState::default();
        let mut cursor = 0usize;

        while cursor < line.len() {
            if state.advance(line, &mut cursor) {
                continue;
            }

            let current = line[cursor..].chars().next().expect("valid cursor");
            if delimiters.contains(&current) {
                count += 1;
            }
            state.previous_significant = Some(current);
            cursor += current.len_utf8();
        }

        count
    }

    #[derive(Default)]
    struct JsScanState {
        previous_significant: Option<char>,
    }

    impl JsScanState {
        fn advance(&mut self, line: &str, cursor: &mut usize) -> bool {
            let rest = &line[*cursor..];
            if rest.starts_with("//") {
                *cursor = line.len();
                return true;
            }
            if rest.starts_with("/*") {
                if let Some(end) = rest.find("*/") {
                    *cursor += end + 2;
                } else {
                    *cursor = line.len();
                }
                return true;
            }

            let Some(current) = rest.chars().next() else {
                return false;
            };
            if matches!(current, '"' | '\'' | '`') {
                *cursor += quoted_js_literal_len(rest, current);
                self.previous_significant = Some(current);
                return true;
            }
            if current == '/' && is_regex_start(self.previous_significant) {
                *cursor += regex_literal_len(rest);
                self.previous_significant = Some('/');
                return true;
            }
            if current.is_whitespace() {
                *cursor += current.len_utf8();
                return true;
            }
            false
        }
    }

    struct JsLineFormatter<'a> {
        input: &'a str,
        cursor: usize,
        output: String,
        previous_significant: Option<char>,
    }

    impl<'a> JsLineFormatter<'a> {
        fn new(input: &'a str) -> Self {
            Self {
                input,
                cursor: 0,
                output: String::new(),
                previous_significant: None,
            }
        }

        fn format(&mut self) -> String {
            while self.cursor < self.input.len() {
                if self.consume_comment_or_literal() {
                    continue;
                }

                let current = self.current_char();
                if current.is_whitespace() {
                    self.consume_whitespace();
                    continue;
                }

                if current.is_ascii_alphabetic() || matches!(current, '_' | '$') {
                    self.consume_identifier();
                    continue;
                }

                if current.is_ascii_digit() {
                    self.consume_number();
                    continue;
                }

                if self.consume_operator() {
                    continue;
                }

                self.push_char(current);
                self.cursor += current.len_utf8();
            }

            normalize_js_output(&self.output)
        }

        fn consume_comment_or_literal(&mut self) -> bool {
            let rest = &self.input[self.cursor..];
            if rest.starts_with("//") || rest.starts_with("/*") {
                self.ensure_space_before_word();
                self.output.push_str(rest.trim_end());
                self.cursor = self.input.len();
                return true;
            }

            let current = self.current_char();
            if matches!(current, '"' | '\'' | '`') {
                let len = quoted_js_literal_len(rest, current);
                self.output.push_str(&rest[..len]);
                self.previous_significant = Some(current);
                self.cursor += len;
                return true;
            }

            if current == '/' && is_regex_start(self.previous_significant) {
                let len = regex_literal_len(rest);
                self.output.push_str(&rest[..len]);
                self.previous_significant = Some('/');
                self.cursor += len;
                return true;
            }

            false
        }

        fn consume_whitespace(&mut self) {
            while self.cursor < self.input.len() && self.current_char().is_whitespace() {
                self.cursor += self.current_char().len_utf8();
            }
            self.mark_soft_space();
        }

        fn consume_identifier(&mut self) {
            let start = self.cursor;
            while self.cursor < self.input.len() {
                let current = self.current_char();
                if current.is_ascii_alphanumeric() || matches!(current, '_' | '$') {
                    self.cursor += current.len_utf8();
                } else {
                    break;
                }
            }

            let ident = &self.input[start..self.cursor];
            self.ensure_space_between_words(ident);
            self.output.push_str(ident);
            self.previous_significant = ident.chars().last();
        }

        fn consume_number(&mut self) {
            while self.cursor < self.input.len() {
                let current = self.current_char();
                if current.is_ascii_alphanumeric() || matches!(current, '_' | '.') {
                    self.output.push(current);
                    self.cursor += current.len_utf8();
                } else {
                    break;
                }
            }
            self.previous_significant = Some('0');
        }

        fn consume_operator(&mut self) -> bool {
            let rest = &self.input[self.cursor..];
            for operator in JS_OPERATORS {
                if rest.starts_with(operator) {
                    self.push_operator(operator);
                    self.cursor += operator.len();
                    return true;
                }
            }
            false
        }

        fn push_operator(&mut self, operator: &str) {
            match operator {
                "." | "?." | "!" | "~" => {
                    self.trim_trailing_space();
                    self.output.push_str(operator);
                }
                "++" | "--" => {
                    self.trim_trailing_space();
                    self.output.push_str(operator);
                }
                "," => {
                    self.trim_trailing_space();
                    self.output.push(',');
                    self.output.push(' ');
                }
                ";" => {
                    self.trim_trailing_space();
                    self.output.push(';');
                    self.output.push(' ');
                }
                ":" if self.in_ternary_context() => {
                    self.push_spaced(operator);
                }
                ":" => {
                    self.trim_trailing_space();
                    self.output.push_str(": ");
                }
                "?" => self.push_spaced(operator),
                "(" | "[" => {
                    self.trim_trailing_space();
                    self.output.push_str(operator);
                }
                ")" | "]" => {
                    self.trim_trailing_space();
                    self.output.push_str(operator);
                }
                "{" => {
                    self.push_spaced("{");
                }
                "}" => {
                    self.trim_trailing_space();
                    if !self.output.is_empty()
                        && !self.output.ends_with('{')
                        && !self.output.ends_with(' ')
                    {
                        self.output.push(' ');
                    }
                    self.output.push('}');
                    self.output.push(' ');
                }
                _ => self.push_spaced(operator),
            }
            self.previous_significant = operator.chars().last();
        }

        fn push_spaced(&mut self, text: &str) {
            self.trim_trailing_space();
            if !self.output.is_empty() {
                self.output.push(' ');
            }
            self.output.push_str(text);
            self.output.push(' ');
        }

        fn push_char(&mut self, current: char) {
            self.output.push(current);
            self.previous_significant = Some(current);
        }

        fn current_char(&self) -> char {
            self.input[self.cursor..]
                .chars()
                .next()
                .expect("valid cursor")
        }

        fn mark_soft_space(&mut self) {
            if !self.output.is_empty() && !self.output.ends_with(' ') {
                self.output.push(' ');
            }
        }

        fn ensure_space_before_word(&mut self) {
            if let Some(previous) = self.output.chars().last() {
                if previous.is_ascii_alphanumeric() || matches!(previous, '_' | '$') {
                    self.output.push(' ');
                }
            }
        }

        fn ensure_space_between_words(&mut self, ident: &str) {
            if let Some(previous) = self.output.chars().last() {
                if (previous.is_ascii_alphanumeric() || matches!(previous, '_' | '$'))
                    && ident
                        .chars()
                        .next()
                        .is_some_and(|current| current.is_ascii_alphanumeric() || current == '_')
                {
                    self.output.push(' ');
                }
            }
        }

        fn trim_trailing_space(&mut self) {
            trim_trailing_space(&mut self.output);
        }

        fn in_ternary_context(&self) -> bool {
            self.output.contains('?')
        }
    }

    const JS_OPERATORS: &[&str] = &[
        "===", "!==", ">>>=", "<<=", ">>=", "**=", "&&=", "||=", "??=", "=>", "++", "--", "**",
        "&&", "||", "??", "?.", "<=", ">=", "==", "!=", "+=", "-=", "*=", "/=", "%=", "&=", "|=",
        "^=", "<<", ">>", "...", "{", "}", "(", ")", "[", "]", ",", ";", ":", "?", ".", "!", "~",
        "<", ">", "=", "+", "-", "*", "/", "%", "&", "|", "^",
    ];

    fn normalize_js_output(output: &str) -> String {
        let mut output = collapse_ws(output.trim());
        for keyword in ["if", "for", "while", "switch", "catch", "with"] {
            output = output.replace(&format!("{keyword}("), &format!("{keyword} ("));
        }
        output = output
            .replace("( ", "(")
            .replace(" )", ")")
            .replace("[ ", "[")
            .replace(" ]", "]")
            .replace("{ }", "{}")
            .replace(" ;", ";")
            .replace(" ,", ",")
            .replace(" .", ".")
            .replace(". ", ".")
            .replace("? .", "?.")
            .replace("? ?", "??");
        output.trim().to_string()
    }

    fn quoted_js_literal_len(input: &str, quote: char) -> usize {
        let mut escaped = false;
        let mut cursor = quote.len_utf8();

        while cursor < input.len() {
            let current = input[cursor..].chars().next().expect("valid cursor");
            cursor += current.len_utf8();
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == quote {
                break;
            }
        }

        cursor
    }

    fn regex_literal_len(input: &str) -> usize {
        let mut escaped = false;
        let mut in_class = false;
        let mut cursor = 1usize;

        while cursor < input.len() {
            let current = input[cursor..].chars().next().expect("valid cursor");
            cursor += current.len_utf8();

            if escaped {
                escaped = false;
                continue;
            }
            match current {
                '\\' => escaped = true,
                '[' => in_class = true,
                ']' => in_class = false,
                '/' if !in_class => break,
                _ => {}
            }
        }

        while cursor < input.len() {
            let current = input[cursor..].chars().next().expect("valid cursor");
            if current.is_ascii_alphabetic() {
                cursor += current.len_utf8();
            } else {
                break;
            }
        }

        cursor
    }

    fn is_regex_start(previous: Option<char>) -> bool {
        previous.is_none_or(|previous| {
            matches!(
                previous,
                '(' | '['
                    | '{'
                    | '='
                    | ':'
                    | ','
                    | ';'
                    | '!'
                    | '?'
                    | '+'
                    | '-'
                    | '*'
                    | '/'
                    | '%'
                    | '&'
                    | '|'
                    | '^'
                    | '~'
                    | '<'
                    | '>'
            )
        })
    }

    fn push_html_tag(lines: &mut Vec<String>, indent: usize, tag: &str, options: FormatOptions) {
        let tag = normalize_html_tag(tag);
        push_line(lines, indent, &tag, options);
    }

    fn push_inline_html(
        lines: &mut Vec<String>,
        indent: usize,
        html: &str,
        options: FormatOptions,
    ) {
        let Some(opening_end) = find_html_tag_end(html) else {
            push_line(lines, indent, &collapse_ws(html.trim()), options);
            return;
        };

        let opening = normalize_html_tag(&html[..=opening_end]);
        let rest = html[opening_end + 1..].trim();

        if let Some(name) = tag_name(&opening) {
            let closing = format!("</{name}>");
            let lower_rest = rest.to_ascii_lowercase();
            if lower_rest.ends_with(&closing) {
                let content_len = rest.len() - closing.len();
                let (content, closing) = rest.split_at(content_len);
                let content = content.trim();
                let closing = closing.trim();

                if content.is_empty() {
                    push_line(lines, indent, &format!("{opening}{closing}"), options);
                    return;
                }

                let content = if is_raw_text_tag(&name) {
                    content.to_string()
                } else {
                    format_inline_content(content, options)
                };
                push_line(
                    lines,
                    indent,
                    &format!("{opening}{content}{closing}"),
                    options,
                );
                return;
            }
        }

        push_line(lines, indent, &collapse_ws(html.trim()), options);
    }

    fn format_inline_content(source: &str, options: FormatOptions) -> String {
        let mut output = String::with_capacity(source.len());
        let mut cursor = 0usize;
        let mut pending_space = false;

        while cursor < source.len() {
            let rest = &source[cursor..];
            if rest.starts_with("<%") {
                let Some(end) = rest.find("%>") else {
                    break;
                };
                push_pending_inline_space(&mut output, pending_space);
                let formatted = format_ejs_tag(&rest[..end + 2], options);
                output.push_str(&collapse_ws(&formatted));
                cursor += end + 2;
                pending_space = false;
                continue;
            }

            if rest.starts_with('<') {
                let Some(end) = find_html_tag_end(rest) else {
                    break;
                };
                let tag = normalize_html_tag(&rest[..=end]);
                if is_closing_tag(&tag) {
                    trim_trailing_space(&mut output);
                } else {
                    push_pending_inline_space(&mut output, pending_space);
                }
                output.push_str(&tag);
                cursor += end + 1;
                pending_space = false;
                continue;
            }

            let next = rest
                .find('<')
                .map(|index| cursor + index)
                .unwrap_or(source.len());
            let text = &source[cursor..next];
            let collapsed = collapse_ws(text);
            if collapsed.is_empty() {
                pending_space |= text.chars().any(char::is_whitespace);
            } else {
                let preserve_leading_space = (pending_space || starts_with_ws(text))
                    && !output_ends_with_unclosed_opening_tag(&output);
                push_pending_inline_space(&mut output, preserve_leading_space);
                output.push_str(&collapsed);
                pending_space = ends_with_ws(text);
            }
            cursor = next;
        }

        output.trim().to_string()
    }

    fn push_pending_inline_space(output: &mut String, pending_space: bool) {
        if pending_space && !output.is_empty() && !output.ends_with(' ') {
            output.push(' ');
        }
    }

    fn starts_with_ws(text: &str) -> bool {
        text.chars().next().is_some_and(char::is_whitespace)
    }

    fn ends_with_ws(text: &str) -> bool {
        text.chars().last().is_some_and(char::is_whitespace)
    }

    fn output_ends_with_unclosed_opening_tag(output: &str) -> bool {
        let Some(tag_start) = output.rfind('<') else {
            return false;
        };
        let tag = output[tag_start..].trim();
        tag.ends_with('>') && !is_closing_tag(tag) && is_opening_tag(tag)
    }

    fn normalize_html_tag(tag: &str) -> String {
        let mut output = String::with_capacity(tag.len());
        let mut cursor = 0usize;
        let mut quote = None;
        let mut pending_space = false;

        while cursor < tag.len() {
            if quote.is_none() && tag[cursor..].starts_with("<%") {
                if pending_space && should_insert_space(&output) {
                    output.push(' ');
                }
                pending_space = false;
                if let Some(end) = tag[cursor + 2..].find("%>") {
                    let end = cursor + end + 4;
                    output.push_str(&tag[cursor..end]);
                    cursor = end;
                    continue;
                }
            }

            let current = tag[cursor..].chars().next().expect("valid cursor");
            match current {
                '"' | '\'' if quote == Some(current) => {
                    quote = None;
                    if pending_space {
                        output.push(' ');
                        pending_space = false;
                    }
                    output.push(current);
                }
                '"' | '\'' if quote.is_none() => {
                    quote = Some(current);
                    if pending_space && should_insert_space(&output) {
                        output.push(' ');
                    }
                    pending_space = false;
                    output.push(current);
                }
                character if character.is_whitespace() && quote.is_none() => {
                    pending_space = true;
                }
                '>' if quote.is_none() => {
                    trim_trailing_space(&mut output);
                    if output.ends_with('/') && !output.ends_with(" /") {
                        output.insert(output.len() - 1, ' ');
                    }
                    output.push('>');
                    pending_space = false;
                }
                _ => {
                    if pending_space && should_insert_space(&output) && current != '/' {
                        output.push(' ');
                    }
                    pending_space = false;
                    output.push(current);
                }
            }
            cursor += current.len_utf8();
        }

        output.trim().to_string()
    }

    fn should_insert_space(output: &str) -> bool {
        !output.is_empty()
            && !output.ends_with('<')
            && !output.ends_with('/')
            && !output.ends_with('=')
    }

    fn trim_trailing_space(output: &mut String) {
        while output.ends_with(' ') {
            output.pop();
        }
    }

    fn push_line(lines: &mut Vec<String>, indent: usize, text: &str, options: FormatOptions) {
        lines.push(format!("{}{}", options.indent_unit().repeat(indent), text));
    }

    fn push_formatted_block(
        lines: &mut Vec<String>,
        indent: usize,
        text: &str,
        options: FormatOptions,
    ) {
        if text.contains('\n') {
            for line in text.lines() {
                push_line(lines, indent, line, options);
            }
        } else {
            push_line(lines, indent, text, options);
        }
    }

    fn collapse_ws(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn is_closing_tag(tag: &str) -> bool {
        tag.starts_with("</")
    }

    fn is_opening_tag(tag: &str) -> bool {
        if tag.starts_with("</")
            || tag.starts_with("<!")
            || tag.starts_with("<?")
            || tag.ends_with("/>")
        {
            return false;
        }

        let name = tag_name(tag).unwrap_or_default();
        !matches!(
            name.as_str(),
            "area"
                | "base"
                | "br"
                | "col"
                | "embed"
                | "hr"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        )
    }

    fn is_raw_text_tag(name: &str) -> bool {
        matches!(name, "script" | "style" | "pre" | "textarea")
    }

    fn is_phrasing_container(name: &str) -> bool {
        is_phrasing_tag(name) || matches!(name, "p" | "dt" | "dd" | "figcaption" | "caption")
    }

    fn is_phrasing_tag(name: &str) -> bool {
        matches!(
            name,
            "a" | "abbr"
                | "audio"
                | "b"
                | "bdi"
                | "bdo"
                | "br"
                | "button"
                | "canvas"
                | "cite"
                | "code"
                | "data"
                | "datalist"
                | "del"
                | "dfn"
                | "em"
                | "embed"
                | "i"
                | "iframe"
                | "img"
                | "input"
                | "ins"
                | "kbd"
                | "label"
                | "mark"
                | "meter"
                | "noscript"
                | "object"
                | "output"
                | "picture"
                | "progress"
                | "q"
                | "ruby"
                | "s"
                | "samp"
                | "script"
                | "select"
                | "slot"
                | "small"
                | "span"
                | "strong"
                | "sub"
                | "sup"
                | "svg"
                | "template"
                | "textarea"
                | "time"
                | "u"
                | "var"
                | "video"
                | "wbr"
        )
    }

    fn is_phrasing_content(source: &str) -> bool {
        let mut cursor = 0usize;
        while cursor < source.len() {
            let rest = &source[cursor..];
            if rest.starts_with("<%") {
                let Some(end) = rest.find("%>") else {
                    return false;
                };
                cursor += end + 2;
                continue;
            }

            if rest.starts_with('<') {
                let Some(relative_end) = find_html_tag_end(rest) else {
                    return false;
                };
                let tag = rest[..=relative_end].trim();
                let Some(name) = tag_name(tag) else {
                    return false;
                };
                if !is_phrasing_tag(&name) {
                    return false;
                }

                let tag_end = cursor + relative_end + 1;
                if is_opening_tag(tag) {
                    let closing = format!("</{name}>");
                    let nested_rest = &source[tag_end..];
                    let Some(closing_start) = nested_rest.find(&closing) else {
                        return false;
                    };
                    let inner = &nested_rest[..closing_start];
                    if !is_phrasing_content(inner) {
                        return false;
                    }
                    cursor = tag_end + closing_start + closing.len();
                } else {
                    cursor = tag_end;
                }
                continue;
            }

            let next = rest
                .find('<')
                .map(|index| cursor + index)
                .unwrap_or(source.len());
            cursor = next;
        }

        true
    }

    fn tag_name(tag: &str) -> Option<String> {
        let name = tag
            .trim_start_matches('<')
            .trim_start_matches('/')
            .split(|character: char| {
                character.is_whitespace() || character == '>' || character == '/'
            })
            .next()?;
        if name.is_empty() {
            None
        } else {
            Some(name.to_ascii_lowercase())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{format_document, FormatOptions};

        fn format_default(input: &str) -> String {
            format_document(input, FormatOptions::default())
        }

        #[test]
        fn formats_html_and_ejs_tags() {
            let input = "<div><%if(users.length){%><span><%=user.name??\"Guest\"%></span><%}else{%><p>No users</p><%}%></div>";
            let output = format_default(input);

            assert!(output.contains("<div>"));
            assert!(output.contains("  <% if (users.length) { %>"));
            assert!(output.contains("    <span><%= user.name ?? \"Guest\" %></span>"));
            assert!(output.contains("  <% } else { %>"));
        }

        #[test]
        fn preserves_front_matter() {
            let input = "---\nlayout: _default\ntitle: 会津若松市\n---\n<main><h1>Top</h1></main>";
            let output = format_default(input);

            assert!(output.starts_with("---\nlayout: _default\ntitle: 会津若松市\n---\n"));
        }

        #[test]
        fn keeps_empty_elements_inline() {
            let input = "<main><div class=\"c-loading__item\"></div><div class=\"c-loading__item\"></div></main>";
            let output = format_default(input);

            assert!(output.contains("  <div class=\"c-loading__item\"></div>"));
            assert_eq!(output.matches("c-loading__item").count(), 2);
        }

        #[test]
        fn keeps_ejs_inside_html_attributes() {
            let input = "<head><title><%= file.data.title %></title><meta name=\"description\" content=\"<%= pageDescription %>\"><link rel=\"canonical\" href=\"<%= pageUrl %>\"></head>";
            let output = format_default(input);

            assert!(output.contains("<title><%= file.data.title %></title>"));
            assert!(
                output.contains("<meta name=\"description\" content=\"<%= pageDescription %>\">")
            );
            assert!(output.contains("<link rel=\"canonical\" href=\"<%= pageUrl %>\">"));
        }

        #[test]
        fn preserves_multiline_js_blocks() {
            let input = "<%\n    // Build canonical path from file's relative position\n    var canonicalPath = '/';\n    var relPath = file.path.replace(/\\\\/g, '/');\n-%>";
            let output = format_default(input);

            assert!(output.contains("// Build canonical path from file's relative position"));
            assert!(output.contains("var canonicalPath = '/';"));
            assert!(output.contains("var relPath = file.path.replace(/\\\\/g, '/');"));
            assert!(!output.contains("/ / Build canonical"));
            assert!(!output.contains("' / '"));
        }

        #[test]
        fn uses_zed_formatting_options_for_indentation() {
            let input = "<main><section><p>Hello</p></section></main>";
            let output = format_document(
                input,
                FormatOptions {
                    tab_size: 4,
                    insert_spaces: true,
                },
            );

            assert!(output.contains("\n    <section>"));
            assert!(output.contains("\n        <p>Hello</p>"));
        }

        #[test]
        fn uses_tabs_when_insert_spaces_is_false() {
            let input = "<main><section><p>Hello</p></section></main>";
            let output = format_document(
                input,
                FormatOptions {
                    tab_size: 4,
                    insert_spaces: false,
                },
            );

            assert!(output.contains("\n\t<section>"));
            assert!(output.contains("\n\t\t<p>Hello</p>"));
        }

        #[test]
        fn keeps_long_html_attributes_on_one_line() {
            let input = "<div class=\"very-long-class-name-that-makes-the-tag-wide\" data-controller=\"navigation menu\" aria-label=\"Primary navigation\"></div>";
            let output = format_default(input);

            assert!(output.contains("<div class=\"very-long-class-name-that-makes-the-tag-wide\" data-controller=\"navigation menu\" aria-label=\"Primary navigation\"></div>"));
            assert!(!output.contains("\n  class=\"very-long-class-name"));
            assert!(!output.contains("\n>"));
        }

        #[test]
        fn keeps_svg_attributes_on_one_line() {
            let input = "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 250 250.04\"><defs><linearGradient id=\"21\" x1=\"17.13\" y1=\"160.35\" x2=\"207.17\" y2=\"160.35\" gradientUnits=\"userSpaceOnUse\"><stop offset=\".62\" stop-color=\"#49a75a\" /></linearGradient></defs><path fill=\"#49a75a\" d=\"M83.45,36.95s-36.73,53.16-22.23,101.49c14.5,48.33,55.09,56.06,55.09,56.06\" /></svg>";
            let output = format_default(input);

            assert!(output.contains("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 250 250.04\">"));
            assert!(output.contains("<linearGradient id=\"21\" x1=\"17.13\" y1=\"160.35\" x2=\"207.17\" y2=\"160.35\" gradientUnits=\"userSpaceOnUse\">"));
            assert!(output.contains("<path fill=\"#49a75a\" d=\"M83.45,36.95s-36.73,53.16-22.23,101.49c14.5,48.33,55.09,56.06,55.09,56.06\" />"));
        }

        #[test]
        fn formats_js_expressions_without_breaking_literals() {
            let input = "<p><%=user.name??fallback.replace(/\\s+/g,\" \").trim()%></p>";
            let output = format_default(input);

            assert!(output.contains("<%= user.name ?? fallback.replace(/\\s+/g, \" \").trim() %>"));
            assert!(!output.contains("/ \\s + /"));
            assert!(!output.contains("\"  \""));
        }

        #[test]
        fn formats_multiline_js_blocks_with_editor_indent() {
            let input = "<%\nif(users.length){\nconst label=users[0]?.name??\"Guest\";\n}else{\nconst label=\"No users\";\n}\n%>";
            let output = format_document(
                input,
                FormatOptions {
                    tab_size: 4,
                    insert_spaces: true,
                },
            );

            assert!(output.contains("<% if (users.length) {"));
            assert!(output.contains("\n    const label = users[0]?.name ?? \"Guest\";"));
            assert!(output.contains("\n} else {"));
            assert!(output.contains("\n    const label = \"No users\";"));
            assert!(output.contains("\n} %>"));
        }

        #[test]
        fn keeps_multiline_ejs_delimiters_with_code() {
            let input = "<head><% if(file.data.vendorcss){\nfile.data.vendorcss.forEach(cssItem => { -%>\n<link rel=\"stylesheet\" href=\"<%= assetsDir %>assets/vendor/<%= cssItem %>.css\">\n<% }); } -%></head>";
            let output = format_default(input);

            assert!(output.contains("  <% if (file.data.vendorcss) {"));
            assert!(output.contains("  file.data.vendorcss.forEach(cssItem => { -%>"));
            assert!(!output.contains("<%\nif"));
            assert!(!output.contains("{\n-%>"));
        }

        #[test]
        fn preserves_js_comments_in_ejs_tags() {
            let input = "<% const url=file.path.replace(/\\\\/g,'/'); // normalize path %>";
            let output = format_default(input);

            assert!(output
                .contains("<% const url = file.path.replace(/\\\\/g, '/'); // normalize path %>"));
            assert!(!output.contains("/ / normalize"));
        }

        #[test]
        fn keeps_nested_inline_html_content_on_one_line() {
            let input = "<section><span class=\"section-ttl__en\">News <span>＆</span> Tips</span></section>";
            let output = format_default(input);

            assert!(output
                .contains("  <span class=\"section-ttl__en\">News <span>＆</span> Tips</span>"));
            assert!(!output.contains("News\n"));
            assert!(!output.contains("\n    Tips"));
        }

        #[test]
        fn keeps_paragraphs_with_phrasing_content_inline() {
            let input = "<div><p>住所<br><span>東京都</span> 千代田区</p></div>";
            let output = format_default(input);

            assert!(output.contains("  <p>住所<br><span>東京都</span> 千代田区</p>"));
            assert!(!output.contains("<p>\n"));
            assert!(!output.contains("<br>\n"));
        }

        #[test]
        fn rejoins_multiline_paragraph_with_single_inline_link() {
            let input = "<div><p class=\"close\">\n  <a href=\"javascript:void(0)\" data-gdpr=\"button\" style=\"display: inline-block;\">\n    Agree\n  </a>\n</p></div>";
            let output = format_default(input);

            assert!(output.contains("<p class=\"close\"><a href=\"javascript:void(0)\" data-gdpr=\"button\" style=\"display: inline-block;\">Agree</a></p>"));
            assert!(!output.contains("<p class=\"close\">\n"));
            assert!(!output.contains(">\n    Agree"));
        }
    }
}
