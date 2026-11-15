// hibana の tree-sitter 文法。色付けのためだけに、トークンを種類に分けて並べる
// ponytail: 構文は解析しない（構文はマクロで増えるため）。関数の呼び出しなどの色を付けるなら LSP の semantic tokens を足す
module.exports = grammar({
  name: "hibana",
  extras: ($) => [/\s/, $.line_comment, $.block_comment],
  word: ($) => $.identifier,
  rules: {
    source_file: ($) => repeat($._token),
    _token: ($) => choice($._plain, $.bracket),
    // { } の対応を見ないトークン
    _plain: ($) =>
      choice(
        $.keyword,
        $.boolean,
        $.primitive_type,
        $.type_identifier,
        $.identifier,
        $.number,
        $.string,
        $.template_string,
        $.operator,
        $.delimiter,
        $.splice,
      ),
    keyword: (_) =>
      choice(
        "fn", "let", "mut", "if", "else", "while", "return", "syntax", "macro", "type", "match",
        "quote", "for", "in", "use", "module", "extern", "effect", "handle", "trait", "impl",
        "struct", "enum",
      ),
    boolean: (_) => choice("true", "false"),
    primitive_type: (_) => choice("int", "bool", "str", "unit", "never", "float"),
    type_identifier: (_) => /[A-Z][A-Za-z0-9_]*/,
    identifier: (_) => /[a-z_][A-Za-z0-9_]*/,
    number: (_) => /0[xX][0-9a-fA-F]+|[0-9]+(\.[0-9]+)?/,
    string: (_) => /'([^'\\]|\\.)*'/,
    // `...{式}...`。{ } の中は式のトークン
    template_string: ($) =>
      seq(
        "`",
        repeat(choice(alias($._template_text, $.string_content), $.escape_sequence, $.interpolation)),
        token.immediate("`"),
      ),
    _template_text: (_) => token.immediate(prec(1, /[^`{\\]+/)),
    escape_sequence: (_) => token.immediate(prec(1, /\\./)),
    interpolation: ($) =>
      seq(alias(token.immediate("{"), $.interpolation_start), repeat($._braced_token), alias("}", $.interpolation_end)),
    // 式の中の { } は対にして読む（式の終わりの } と区別するため）
    _braced_token: ($) => choice($._plain, alias(choice("(", ")", "[", "]"), $.bracket), $._braces),
    _braces: ($) => seq(alias("{", $.bracket), repeat($._braced_token), alias("}", $.bracket)),
    splice: (_) => /\$[A-Za-z_][A-Za-z0-9_]*|\$/,
    operator: (_) =>
      choice("->", "=>", "..", "==", "!=", "<=", ">=", "&&", "||", "</", "=", "<", ">", "+", "-", "*", "/", "%", "!", "@", "~", "|", "?"),
    bracket: (_) => choice("(", ")", "{", "}", "[", "]"),
    delimiter: (_) => choice("::", ",", ";", ":", "."),
    line_comment: (_) => token(seq("//", /[^\n]*/)),
    block_comment: (_) => token(seq("/*", /[^*]*\*+([^/*][^*]*\*+)*/, "/")),
  },
});
