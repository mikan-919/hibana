// hibana の tree-sitter 文法。色付けのためだけに、トークンを種類に分けて並べる
// ponytail: 構文は解析しない（構文はマクロで増えるため）。関数の呼び出しなどの色を付けるなら LSP の semantic tokens を足す
module.exports = grammar({
  name: "hibana",
  extras: ($) => [/\s/, $.line_comment, $.block_comment],
  word: ($) => $.identifier,
  rules: {
    source_file: ($) => repeat($._token),
    _token: ($) =>
      choice(
        $.keyword,
        $.boolean,
        $.primitive_type,
        $.type_identifier,
        $.identifier,
        $.number,
        $.string,
        $.char,
        $.operator,
        $.bracket,
        $.delimiter,
        $.splice,
      ),
    keyword: (_) =>
      choice(
        "fn", "let", "mut", "if", "else", "while", "return", "syntax", "macro", "type", "match",
        "quote", "partial", "for", "in", "use", "module", "extern", "effect", "handle",
      ),
    boolean: (_) => choice("true", "false"),
    primitive_type: (_) => choice("int", "bool", "str", "unit", "never", "float"),
    type_identifier: (_) => /[A-Z][A-Za-z0-9_]*/,
    identifier: (_) => /[a-z_][A-Za-z0-9_]*/,
    number: (_) => /0[xX][0-9a-fA-F]+|[0-9]+(\.[0-9]+)?/,
    string: (_) => /"([^"\\]|\\.)*"/,
    char: (_) => /'([^'\\]|\\.)'/,
    splice: (_) => /\$[A-Za-z_][A-Za-z0-9_]*|\$/,
    operator: (_) =>
      choice("->", "=>", "..", "==", "!=", "<=", ">=", "&&", "||", "</", "=", "<", ">", "+", "-", "*", "/", "%", "!", "@", "~", "|", "?"),
    bracket: (_) => choice("(", ")", "{", "}", "[", "]"),
    delimiter: (_) => choice(",", ";", ":", "."),
    line_comment: (_) => token(seq("//", /[^\n]*/)),
    block_comment: (_) => token(seq("/*", /[^*]*\*+([^/*][^*]*\*+)*/, "/")),
  },
});
