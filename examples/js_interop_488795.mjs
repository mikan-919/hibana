import { add as $imp_util$_add } from "./js/util.ts";
import { shout as $imp_util$_shout } from "./js/util.ts";
import { applyTwice as $imp_util$_applyTwice } from "./js/util.ts";
import { point as $imp_util$_point } from "./js/util.ts";
import { words as $imp_util$_words } from "./js/util.ts";
import { later as $imp_util$_later } from "./js/util.ts";
import { mapLater as $imp_util$_mapLater } from "./js/util.ts";
import { hello as $imp_greet$_hello } from "./js/greet.js";
import { size as $imp_greet$_size } from "./js/greet.js";
"use strict";
const $MIN = -(2n ** 63n), $MAX = 2n ** 63n - 1n;
function $chk(x) { if (x < $MIN || x > $MAX) throw new Error("オーバーフロー"); return x; }
function $add(a, b) { return typeof a === "string" ? a + b : $chk(a + b); }
function $sub(a, b) { return $chk(a - b); }
function $mul(a, b) { return $chk(a * b); }
function $div(a, b) { if (b === 0n) throw new Error("0 で除算した"); return $chk(a / b); }
function $rem(a, b) { if (b === 0n) throw new Error("0 で除算した"); return a % b; }
function $neg(a) { return $chk(-a); }
let $w;
// 最後に通った、失敗しうる式の位置
let $at;
// 書き換えられる参照
class $Ref { constructor(v) { this.v = v; } }
function $d(c, f) { return { $c: c, $f: f }; }
function $field(x, f) { return x.$f[$fields[x.$c].indexOf(f)]; }
function $eq(a, b) {
  if (a === b) return true;
  if (a instanceof $Ref) return false;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => $eq(x, b[i]));
  if (a && b && typeof a === "object" && typeof b === "object") return a.$c === b.$c && $eq(a.$f, b.$f);
  return false;
}
function $show(v) {
  if (typeof v === "bigint" || typeof v === "boolean") return String(v);
  if (typeof v === "string") return JSON.stringify(v);
  if (v === undefined) return "()";
  if (typeof v === "function") return "<fn>";
  if (v instanceof $Ref) return "<ref>";
  if (Array.isArray(v)) return "[" + v.map($show).join(", ") + "]";
  if (v.$c.endsWith("{}") && $fields[v.$c][0] === "0") return v.$c.slice(0, -2) + "(" + v.$f.map($show).join(", ") + ")";
  if (v.$c.endsWith("{}")) return v.$c.slice(0, -2) + " { " + $fields[v.$c].map((f, i) => f + ": " + $show(v.$f[i])).join(", ") + " }";
  return v.$f.length === 0 ? v.$c : v.$c + "(" + v.$f.map($show).join(", ") + ")";
}
// web ライブラリの serve と worker。app は JS の関数（$to で変換済み）で、{ method, path, body } を受け取る
globalThis.hibana_serve = async (port, app) => {
  const http = await import("node:http");
  const server = http.createServer(async (req, res) => {
    try {
      let body = "";
      for await (const c of req) body += c;
      const u = new URL(req.url, "http://localhost");
      const headers = Object.entries(req.headers).map(([name, value]) => ({ name, value: String(value) }));
      const r = await app({ method: req.method, path: u.pathname, query: u.search.slice(1), headers, body });
      res.writeHead(r.status, { "content-type": r.content_type });
      res.end(r.body);
    } catch (e) {
      console.error("実行時エラー: " + e.message + ($at ? "（" + $at + "）" : ""));
      res.writeHead(500, { "content-type": "text/plain" });
      res.end("Internal Server Error");
    }
  });
  await new Promise((ok) => server.listen(port, ok));
  console.log("http://localhost:" + server.address().port);
};
let $worker;
globalThis.hibana_worker = (app) => { $worker = app; };
const $fetch = async (req) => {
  const u = new URL(req.url);
  const headers = [...req.headers].map(([name, value]) => ({ name, value }));
  const r = await $worker({ method: req.method, path: u.pathname, query: u.search.slice(1), headers, body: await req.text() });
  return new Response(r.body, { status: r.status, headers: { "content-type": r.content_type } });
};
// 処理中の handle。内側ほど後ろ。腕の中の操作は、その handle より外で処理する
// 並べ替えずに作り直す（並行に動く処理と共有するため）。await の後は $k で、待つ前の並びに戻す
let $h = [];
function $k(h, v) { $h = h; return v; }
// all。すべて終わるのを待ち、失敗したものがあれば最初の失敗を投げる
// ponytail: 失敗しても残りを中断しない（終わるまで待つだけ）。中断するなら AbortSignal を各処理に渡す
async function $all(fs) {
  const h = $h;
  const rs = await Promise.allSettled(fs.map((f) => { $h = h; return f(); }));
  $h = h;
  const bad = rs.find((r) => r.status === "rejected");
  if (bad) throw bad.reason;
  return rs.map((r) => r.value);
}
class $Abort { constructor(h, v) { this.h = h; this.v = v; } }
function $op(op, args) {
  for (let i = $h.length - 1; i >= 0; i--) {
    const h = $h[i];
    if (!Object.hasOwn(h.arms, op)) continue;
    const saved = $h;
    $h = $h.slice(0, i);
    let r;
    try { r = h.arms[op](...args); } finally { $h = saved; }
    if (r && r.$c === "$resume") return r.$f[0];
    throw new $Abort(h, r);
  }
  throw new Error("操作 " + op + " を処理する handle が無い");
}
function $str(v) { return typeof v === "string" ? v : $show(v); }
function $print(v) { console.log($str(v)); }
function $len(v) { return BigInt(typeof v === "string" ? [...v].length : v.length); }
function $split(s, sep) { return s.split(sep); }
function $starts_with(s, p) { return s.startsWith(p); }
// x = push(x, e)。$o は x の変数だけが指している配列の印で、x を他で読むと $sh が消す
function $sh(v) { if (v !== null && v !== undefined && v.$o) v.$o = false; return v; }
function $opush(v, x) {
  if (v.$o) { v.push(x); return v; }
  const c = [...v, x]; c.$o = true; return c;
}
function $chars(s) { return [...s]; }
function $code(s) { return s.length ? BigInt(s.codePointAt(0)) : -1n; }
function $panic(m) { throw new Error(m); }
function $slice(s, i, j) {
  const cs = [...s];
  if (!(0n <= i && i <= j && j <= BigInt(cs.length))) throw new Error(`slice の範囲 ${i}..${j} が文字列の外`);
  return cs.slice(Number(i), Number(j)).join("");
}
// extern の関数の境界で、宣言した型 d に従って値を変換する。$to は hibana から JS へ、$from は JS から hibana へ
function $to(d, v) {
  if (d === "int") return Number(v);
  if (d.vec) return v.map((x) => $to(d.vec, x));
  if (d.fn && d.async) return (...a) => Promise.resolve(v(...a.map((x, i) => $from(d.fn[i], x)))).then((r) => $to(d.ret, r));
  if (d.fn) return (...a) => $to(d.ret, v(...a.map((x, i) => $from(d.fn[i], x))));
  if (d.st) return Object.fromEntries(d.f.map(([n, fd], i) => [n, $to(fd, v.$f[i])]));
  return v;
}
function $from(d, v) {
  if (d === "int") {
    if (typeof v === "bigint") return $chk(v);
    if (!Number.isInteger(v)) throw new Error("JavaScript の値 " + v + " は int でない");
    return BigInt(v);
  }
  if (d === "unit") return undefined;
  if (d.vec) return Array.from(v, (x) => $from(d.vec, x));
  if (d.fn && d.async) return async (...a) => $from(d.ret, await v(...a.map((x, i) => $to(d.fn[i], x))));
  if (d.fn) return (...a) => $from(d.ret, v(...a.map((x, i) => $to(d.fn[i], x))));
  if (d.st) return $d(d.st, d.f.map(([n, fd]) => $from(fd, v[n])));
  return v;
}
const $fields = { "util.Point{}": ["x", "y"], "greet.Size{}": ["w", "h"], "std.None": [], "std.Some": ["value"], "std.Ok": ["value"], "std.Err": ["error"], "std.Entry{}": ["key", "value"], "std.Map{}": ["entries"], "std.JNull": [], "std.JBool": ["b"], "std.JInt": ["n"], "std.JStr": ["s"], "std.JArr": ["items"], "std.JObj": ["fields"], "std.Field{}": ["key", "value"], "std.Parsed{}": ["ok", "value", "pos"], "std.Eq{}": ["eq"], "std.Ord{}": ["super#std.Eq", "lt"], "std.Show{}": ["show"] };
const f_tag = function() {
return undefined;
};
const f_std$_eq = function(v_A_1, v_a_2, v_b_3, v_inst$0_4) {
const t5 = $field(v_inst$0_4, "eq");
const t6 = t5(v_a_2, v_b_3);
return t6;
};
const f_std$_Eq$get = function(v_A_7, v_inst$0_8) {
return v_inst$0_8;
};
const f_std$_lt = function(v_A_9, v_a_10, v_b_11, v_inst$0_12) {
const t13 = $field(v_inst$0_12, "lt");
const t14 = t13(v_a_10, v_b_11);
return t14;
};
const f_std$_Ord$get = function(v_A_15, v_inst$0_16) {
return v_inst$0_16;
};
const f_std$_show = function(v_A_17, v_x_18, v_inst$0_19) {
const t20 = $field(v_inst$0_19, "show");
const t21 = t20(v_x_18);
return t21;
};
const f_std$_Show$get = function(v_A_22, v_inst$0_23) {
return v_inst$0_23;
};
const f_twice_later = async function(v_x_24) {
$at = "9 行 5 列";
$at = "9 行 5 列";
const t25 = $k($h, await f_util$_later(v_x_24));
$at = "9 行 21 列";
const t26 = $k($h, await f_util$_later(v_x_24));
const t27 = $add(t25, t26);
return t27;
};
const f_job = async function(v_name_28, v_n_29) {
const t31 = (() => function() {
const t30 = $d("$resume", [v_name_28]);
return t30;
})();
const t32 = { arms: { "tag": t31 } };
let t33;
const t34 = $h; $h = [...$h, t32];
try {
let t35;
{
$at = "19 行 17 列";
const t36 = $op("tag", []);
let v_a_37 = t36;
$at = "20 行 17 列";
const t38 = $k($h, await f_util$_later(v_n_29));
let v_x_39 = t38;
$at = "21 行 9 列";
$at = "21 行 9 列";
const t40 = $str(v_x_39);
const t41 = $add(v_a_37, t40);
$at = "21 行 25 列";
const t42 = $op("tag", []);
const t43 = $add(t41, t42);
t35 = t43;
}
t33 = t35;
} catch (e) { if (e instanceof $Abort && e.h === t32) t33 = e.v; else throw e; } finally { $h = t34; }
return t33;
};
const f_main = async function() {
$at = "28 行 11 列";
$at = "28 行 11 列";
const t44 = f_util$_add(2n, 3n);
const t45 = $mul(t44, 10n);
const t46 = $print(t45);
$at = "29 行 11 列";
const t47 = f_util$_shout("hibana");
const t48 = $print(t47);
$at = "30 行 11 列";
const t51 = (() => function(v_x_49) {
$at = "30 行 35 列";
const t50 = $mul(v_x_49, 3n);
return t50;
})();
const t52 = f_util$_applyTwice(t51, 2n);
const t53 = $print(t52);
$at = "31 行 13 列";
const t54 = f_util$_point(1n, 2n);
let v_p_55 = t54;
$at = "32 行 11 列";
const t56 = $field(v_p_55, "x");
const t57 = $field(v_p_55, "y");
const t58 = $add(t56, t57);
const t59 = $print(t58);
$at = "33 行 15 列";
const t60 = f_util$_words("a b c");
const t61 = $len(t60);
const t62 = $print(t61);
$at = "34 行 11 列";
const t63 = f_greet$_hello("world");
const t64 = $print(t63);
const t65 = $d("greet.Size{}", [3n, 4n]);
const t66 = f_greet$_size(t65);
const t67 = $field(t66, "w");
const t68 = $print(t67);
const t69 = $k($h, await f_twice_later(5n));
const t70 = $print(t69);
$at = "38 行 11 列";
const t71 = [1n, 2n];
const t75 = (() => async function(v_x_72) {
$at = "38 行 41 列";
$at = "38 行 41 列";
const t73 = $k($h, await f_util$_later(v_x_72));
const t74 = $add(t73, 1n);
return t74;
})();
const t76 = $k($h, await f_util$_mapLater(t71, t75));
const t77 = $print(t76);
const t79 = (() => async function() {
const t78 = $k($h, await f_job("a", 1n));
return t78;
})();
const t81 = (() => async function() {
const t80 = $k($h, await f_job("b", 2n));
return t80;
})();
const t82 = [t79, t81];
const t83 = $k($h, await $all(t82));
const t84 = $print(t83);
const t85 = [1n, 2n];
const t89 = (() => async function(v_x_86) {
$at = "41 行 31 列";
$at = "41 行 31 列";
const t87 = $k($h, await f_util$_later(v_x_86));
const t88 = $mul(t87, 10n);
return t88;
})();
const t90 = $k($h, await f_std$_map$a(undefined, undefined, undefined, t85, t89));
const t91 = $print(t90);
return t91;
};
const f_std$_max = function(v_A_92, v_a_93, v_b_94, v_inst$0_95) {
const t96 = f_std$_lt(undefined, v_a_93, v_b_94, v_inst$0_95);
let t97;
if (t96)
{
t97 = v_b_94;
}
else
{
let t98;
{
t98 = v_a_93;
}
t97 = t98;
}
return t97;
};
const f_std$_min = function(v_A_99, v_a_100, v_b_101, v_inst$0_102) {
const t103 = f_std$_lt(undefined, v_b_101, v_a_100, v_inst$0_102);
let t104;
if (t103)
{
t104 = v_b_101;
}
else
{
let t105;
{
t105 = v_a_100;
}
t104 = t105;
}
return t104;
};
const f_std$_contains = function(v_A_106, v_xs_107, v_x_108, v_inst$0_109) {
for (const t110 of v_xs_107) {
const v_y_111 = t110;
const t112 = f_std$_eq(undefined, v_x_108, v_y_111, v_inst$0_109);
let t113;
if (t112)
{
return true;
t113 = undefined;
}
}
return false;
};
const f_std$_assert = function(v_ok_114) {
const t115 = !v_ok_114;
let t116;
if (t115)
{
$at = "246 行 14 列";
const t117 = $panic("assert が失敗した");
t116 = t117;
}
return t116;
};
const f_std$_assert_eq = function(v_A_118, v_got_119, v_want_120, v_inst$0_121, v_inst$1_122) {
const t123 = f_std$_eq(undefined, v_got_119, v_want_120, v_inst$0_121);
const t124 = !t123;
let t125;
if (t124)
{
$at = "250 行 22 列";
$at = "250 行 28 列";
$at = "250 行 28 列";
$at = "250 行 28 列";
const t126 = f_std$_show(undefined, v_got_119, v_inst$1_122);
const t127 = $add("assert_eq が失敗した: ", t126);
const t128 = $add(t127, " と ");
const t129 = f_std$_show(undefined, v_want_120, v_inst$1_122);
const t130 = $add(t128, t129);
const t131 = $panic(t130);
t125 = t131;
}
return t125;
};
const f_std$_unwrap_or = function(v_A_132, v_o_133, v_d_134) {
let t135;
switch (v_o_133.$c) {
case "std.Some": {
const v_x_136 = v_o_133.$f[0];
t135 = v_x_136;
break;
}
case "std.None": {
t135 = v_d_134;
break;
}
}
return t135;
};
const f_std$_map = function(v_A_137, v_B_138, v_E_139, v_xs_140, v_f_141) {
const t142 = [];
let v_out_143 = t142;
for (const t144 of v_xs_140) {
const v_x_145 = t144;
const t146 = v_f_141(v_x_145);
v_out_143 = $opush(v_out_143, t146);
}
return $sh(v_out_143);
};
const f_std$_map$a = async function(v_A_147, v_B_148, v_E_149, v_xs_150, v_f_151) {
const t152 = [];
let v_out_153 = t152;
for (const t154 of v_xs_150) {
const v_x_155 = t154;
const t156 = $k($h, await v_f_151(v_x_155));
v_out_153 = $opush(v_out_153, t156);
}
return $sh(v_out_153);
};
const f_std$_filter = function(v_A_157, v_E_158, v_xs_159, v_f_160) {
const t161 = [];
let v_out_162 = t161;
for (const t163 of v_xs_159) {
const v_x_164 = t163;
const t165 = v_f_160(v_x_164);
let t166;
if (t165)
{
v_out_162 = $opush(v_out_162, v_x_164);
t166 = undefined;
}
}
return $sh(v_out_162);
};
const f_std$_filter$a = async function(v_A_167, v_E_168, v_xs_169, v_f_170) {
const t171 = [];
let v_out_172 = t171;
for (const t173 of v_xs_169) {
const v_x_174 = t173;
const t175 = $k($h, await v_f_170(v_x_174));
let t176;
if (t175)
{
v_out_172 = $opush(v_out_172, v_x_174);
t176 = undefined;
}
}
return $sh(v_out_172);
};
const f_std$_fold = function(v_A_177, v_B_178, v_E_179, v_xs_180, v_init_181, v_f_182) {
let v_acc_183 = v_init_181;
for (const t184 of v_xs_180) {
const v_x_185 = t184;
const t186 = v_f_182($sh(v_acc_183), v_x_185);
v_acc_183 = t186;
}
return $sh(v_acc_183);
};
const f_std$_fold$a = async function(v_A_187, v_B_188, v_E_189, v_xs_190, v_init_191, v_f_192) {
let v_acc_193 = v_init_191;
for (const t194 of v_xs_190) {
const v_x_195 = t194;
const t196 = $k($h, await v_f_192($sh(v_acc_193), v_x_195));
v_acc_193 = t196;
}
return $sh(v_acc_193);
};
const f_std$_empty = function(v_V_197) {
const t198 = [];
let v_es_199 = t198;
const t200 = $d("std.Map{}", [v_es_199]);
return t200;
};
const f_std$_get = function(v_V_201, v_m_202, v_key_203) {
const t204 = $d("std.None", []);
let v_r_205 = t204;
const t206 = $field(v_m_202, "entries");
for (const t207 of t206) {
const v_e_208 = t207;
const t209 = $field(v_e_208, "key");
const t210 = $eq(t209, v_key_203);
let t211;
if (t210)
{
const t212 = $field(v_e_208, "value");
const t213 = $d("std.Some", [t212]);
v_r_205 = t213;
t211 = undefined;
}
}
return $sh(v_r_205);
};
const f_std$_set = function(v_V_214, v_m_215, v_key_216, v_value_217) {
const t218 = [];
let v_es_219 = t218;
const t220 = $field(v_m_215, "entries");
for (const t221 of t220) {
const v_e_222 = t221;
const t223 = $field(v_e_222, "key");
const t224 = !$eq(t223, v_key_216);
let t225;
if (t224)
{
v_es_219 = $opush(v_es_219, v_e_222);
t225 = undefined;
}
}
const t226 = $d("std.Entry{}", [v_key_216, v_value_217]);
const t227 = [...$sh(v_es_219), t226];
const t228 = $d("std.Map{}", [t227]);
return t228;
};
const f_std$_keys = function(v_V_229, v_m_230) {
const t231 = [];
let v_ks_232 = t231;
const t233 = $field(v_m_230, "entries");
for (const t234 of t233) {
const v_e_235 = t234;
const t236 = $field(v_e_235, "key");
v_ks_232 = $opush(v_ks_232, t236);
}
return $sh(v_ks_232);
};
const f_std$_quote_str = function(v_s_237) {
let v_out_238 = "\"";
const t239 = $len(v_s_237);
for (let t240 = 0n; t240 < t239; t240++) {
const v_i_241 = t240;
$at = "326 行 17 列";
$at = "326 行 29 列";
const t242 = $add(v_i_241, 1n);
const t243 = $slice(v_s_237, v_i_241, t242);
let v_c_244 = t243;
const t245 = $eq(v_c_244, "\"");
let t246;
if (t245)
{
$at = "328 行 19 列";
const t247 = $add($sh(v_out_238), "\\\"");
v_out_238 = t247;
t246 = undefined;
}
else
{
const t248 = $eq(v_c_244, "\\");
let t249;
if (t248)
{
$at = "330 行 19 列";
const t250 = $add($sh(v_out_238), "\\\\");
v_out_238 = t250;
t249 = undefined;
}
else
{
const t251 = $eq(v_c_244, "\n");
let t252;
if (t251)
{
$at = "332 行 19 列";
const t253 = $add($sh(v_out_238), "\\n");
v_out_238 = t253;
t252 = undefined;
}
else
{
let t254;
{
$at = "334 行 19 列";
const t255 = $add($sh(v_out_238), v_c_244);
v_out_238 = t255;
t254 = undefined;
}
t252 = t254;
}
t249 = t252;
}
t246 = t249;
}
}
$at = "337 行 5 列";
const t256 = $add($sh(v_out_238), "\"");
return t256;
};
const f_std$_stringify = function(v_j_257) {
let t258;
switch (v_j_257.$c) {
case "std.JNull": {
t258 = "null";
break;
}
case "std.JBool": {
const v_b_259 = v_j_257.$f[0];
const t260 = $str(v_b_259);
t258 = t260;
break;
}
case "std.JInt": {
const v_n_261 = v_j_257.$f[0];
const t262 = $str(v_n_261);
t258 = t262;
break;
}
case "std.JStr": {
const v_s_263 = v_j_257.$f[0];
const t264 = f_std$_quote_str(v_s_263);
t258 = t264;
break;
}
case "std.JArr": {
const v_items_265 = v_j_257.$f[0];
let t266;
{
let v_out_267 = "[";
let v_first_268 = true;
for (const t269 of v_items_265) {
const v_x_270 = t269;
const t271 = !$sh(v_first_268);
let t272;
if (t271)
{
$at = "351 行 35 列";
const t273 = $add($sh(v_out_267), ",");
v_out_267 = t273;
t272 = undefined;
}
$at = "352 行 23 列";
const t274 = f_std$_stringify(v_x_270);
const t275 = $add($sh(v_out_267), t274);
v_out_267 = t275;
v_first_268 = false;
}
$at = "355 行 13 列";
const t276 = $add($sh(v_out_267), "]");
t266 = t276;
}
t258 = t266;
break;
}
case "std.JObj": {
const v_fields_277 = v_j_257.$f[0];
let t278;
{
let v_out_279 = "{";
let v_first_280 = true;
for (const t281 of v_fields_277) {
const v_f_282 = t281;
const t283 = !$sh(v_first_280);
let t284;
if (t283)
{
$at = "361 行 35 列";
const t285 = $add($sh(v_out_279), ",");
v_out_279 = t285;
t284 = undefined;
}
$at = "362 行 23 列";
$at = "362 行 23 列";
$at = "362 行 23 列";
const t286 = $field(v_f_282, "key");
const t287 = f_std$_quote_str(t286);
const t288 = $add($sh(v_out_279), t287);
const t289 = $add(t288, ":");
const t290 = $field(v_f_282, "value");
const t291 = f_std$_stringify(t290);
const t292 = $add(t289, t291);
v_out_279 = t292;
v_first_280 = false;
}
$at = "365 行 13 列";
const t293 = $add($sh(v_out_279), "}");
t278 = t293;
}
t258 = t278;
break;
}
}
return t258;
};
const f_std$_no_parse = function() {
const t294 = $d("std.JNull", []);
const t295 = $d("std.Parsed{}", [false, t294, 0n]);
return t295;
};
const f_std$_char_at = function(v_s_296, v_i_297) {
const t298 = (0n <= v_i_297);
let t299 = t298;
if (t299)
{
const t300 = $len(v_s_296);
const t301 = (v_i_297 < t300);
t299 = t301;
}
let t302;
if (t299)
{
$at = "381 行 31 列";
$at = "381 行 43 列";
const t303 = $add(v_i_297, 1n);
const t304 = $slice(v_s_296, v_i_297, t303);
t302 = t304;
}
else
{
let t305;
{
t305 = "";
}
t302 = t305;
}
return t302;
};
const f_std$_skip_ws = function(v_s_306, v_i_307) {
let v_j_308 = v_i_307;
while (true) {
const t309 = f_std$_char_at(v_s_306, $sh(v_j_308));
const t310 = $eq(t309, " ");
let t311 = t310;
if (!t311)
{
const t312 = f_std$_char_at(v_s_306, $sh(v_j_308));
const t313 = $eq(t312, "\n");
t311 = t313;
}
let t314 = t311;
if (!t314)
{
const t315 = f_std$_char_at(v_s_306, $sh(v_j_308));
const t316 = $eq(t315, "\t");
t314 = t316;
}
let t317 = t314;
if (!t317)
{
const t318 = f_std$_char_at(v_s_306, $sh(v_j_308));
const t319 = $eq(t318, "\r");
t317 = t319;
}
if (!t317) break;
$at = "387 行 13 列";
const t320 = $add($sh(v_j_308), 1n);
v_j_308 = t320;
}
return $sh(v_j_308);
};
const f_std$_digit = function(v_c_321) {
let v_ds_322 = "0123456789";
$at = "395 行 17 列";
const t323 = $neg(1n);
let v_r_324 = t323;
for (let t325 = 0n; t325 < 10n; t325++) {
const v_k_326 = t325;
$at = "397 行 12 列";
$at = "397 行 25 列";
const t327 = $add(v_k_326, 1n);
const t328 = $slice(v_ds_322, v_k_326, t327);
const t329 = $eq(t328, v_c_321);
let t330;
if (t329)
{
v_r_324 = v_k_326;
t330 = undefined;
}
}
return $sh(v_r_324);
};
const f_std$_starts_at = function(v_s_331, v_i_332, v_w_333) {
const t334 = (0n <= v_i_332);
let t335 = t334;
if (t335)
{
$at = "403 行 15 列";
const t336 = $len(v_w_333);
const t337 = $add(v_i_332, t336);
const t338 = $len(v_s_331);
const t339 = (t337 <= t338);
t335 = t339;
}
let t340 = t335;
if (t340)
{
$at = "403 行 39 列";
$at = "403 行 51 列";
const t341 = $len(v_w_333);
const t342 = $add(v_i_332, t341);
const t343 = $slice(v_s_331, v_i_332, t342);
const t344 = $eq(t343, v_w_333);
t340 = t344;
}
return t340;
};
const f_std$_parse_int = function(v_s_345, v_i_346) {
let v_j_347 = v_i_346;
const t348 = f_std$_char_at(v_s_345, $sh(v_j_347));
const t349 = $eq(t348, "-");
let v_neg_350 = t349;
let t351;
if (v_neg_350)
{
$at = "410 行 18 列";
const t352 = $add($sh(v_j_347), 1n);
v_j_347 = t352;
t351 = undefined;
}
let v_start_353 = $sh(v_j_347);
let v_n_354 = 0n;
while (true) {
const t355 = f_std$_char_at(v_s_345, $sh(v_j_347));
const t356 = f_std$_digit(t355);
const t357 = (t356 >= 0n);
if (!t357) break;
$at = "414 行 13 列";
$at = "414 行 13 列";
const t358 = $mul($sh(v_n_354), 10n);
const t359 = f_std$_char_at(v_s_345, $sh(v_j_347));
const t360 = f_std$_digit(t359);
const t361 = $add(t358, t360);
v_n_354 = t361;
$at = "415 行 13 列";
const t362 = $add($sh(v_j_347), 1n);
v_j_347 = t362;
}
const t363 = $eq($sh(v_j_347), v_start_353);
let t364;
if (t363)
{
const t365 = f_std$_no_parse();
t364 = t365;
}
else
{
let t366;
{
let t367;
if (v_neg_350)
{
$at = "420 行 49 列";
const t368 = $sub(0n, $sh(v_n_354));
t367 = t368;
}
else
{
let t369;
{
t369 = $sh(v_n_354);
}
t367 = t369;
}
const t370 = $d("std.JInt", [t367]);
const t371 = $d("std.Parsed{}", [true, t370, $sh(v_j_347)]);
t366 = t371;
}
t364 = t366;
}
return t364;
};
const f_std$_parse_string = function(v_s_372, v_i_373) {
$at = "426 行 17 列";
const t374 = $add(v_i_373, 1n);
let v_j_375 = t374;
let v_out_376 = "";
let v_done_377 = false;
let v_bad_378 = false;
while (true) {
const t379 = !$sh(v_done_377);
let t380 = t379;
if (t380)
{
const t381 = !$sh(v_bad_378);
t380 = t381;
}
if (!t380) break;
const t382 = f_std$_char_at(v_s_372, $sh(v_j_375));
let v_c_383 = t382;
const t384 = $eq(v_c_383, "");
let t385;
if (t384)
{
v_bad_378 = true;
t385 = undefined;
}
else
{
const t386 = $eq(v_c_383, "\"");
let t387;
if (t386)
{
v_done_377 = true;
$at = "436 行 17 列";
const t388 = $add($sh(v_j_375), 1n);
v_j_375 = t388;
t387 = undefined;
}
else
{
const t389 = $eq(v_c_383, "\\");
let t390;
if (t389)
{
$at = "438 行 32 列";
const t391 = $add($sh(v_j_375), 1n);
const t392 = f_std$_char_at(v_s_372, t391);
let v_e_393 = t392;
const t394 = $eq(v_e_393, "n");
let t395;
if (t394)
{
$at = "440 行 23 列";
const t396 = $add($sh(v_out_376), "\n");
v_out_376 = t396;
t395 = undefined;
}
else
{
const t397 = $eq(v_e_393, "t");
let t398;
if (t397)
{
$at = "442 行 23 列";
const t399 = $add($sh(v_out_376), "\t");
v_out_376 = t399;
t398 = undefined;
}
else
{
const t400 = $eq(v_e_393, "\"");
let t401 = t400;
if (!t401)
{
const t402 = $eq(v_e_393, "\\");
t401 = t402;
}
let t403 = t401;
if (!t403)
{
const t404 = $eq(v_e_393, "/");
t403 = t404;
}
let t405;
if (t403)
{
$at = "444 行 23 列";
const t406 = $add($sh(v_out_376), v_e_393);
v_out_376 = t406;
t405 = undefined;
}
else
{
let t407;
{
v_bad_378 = true;
t407 = undefined;
}
t405 = t407;
}
t398 = t405;
}
t395 = t398;
}
$at = "448 行 17 列";
const t408 = $add($sh(v_j_375), 2n);
v_j_375 = t408;
t390 = undefined;
}
else
{
let t409;
{
$at = "450 行 19 列";
const t410 = $add($sh(v_out_376), v_c_383);
v_out_376 = t410;
$at = "451 行 17 列";
const t411 = $add($sh(v_j_375), 1n);
v_j_375 = t411;
t409 = undefined;
}
t390 = t409;
}
t387 = t390;
}
t385 = t387;
}
}
let t412;
if ($sh(v_bad_378))
{
const t413 = f_std$_no_parse();
t412 = t413;
}
else
{
let t414;
{
const t415 = $d("std.JStr", [$sh(v_out_376)]);
const t416 = $d("std.Parsed{}", [true, t415, $sh(v_j_375)]);
t414 = t416;
}
t412 = t414;
}
return t412;
};
const f_std$_parse_value = function(v_s_417, v_i0_418) {
const t419 = f_std$_skip_ws(v_s_417, v_i0_418);
let v_i_420 = t419;
const t421 = f_std$_char_at(v_s_417, v_i_420);
let v_c_422 = t421;
const t423 = f_std$_starts_at(v_s_417, v_i_420, "null");
let t424;
if (t423)
{
const t425 = $d("std.JNull", []);
$at = "461 行 47 列";
const t426 = $add(v_i_420, 4n);
const t427 = $d("std.Parsed{}", [true, t425, t426]);
t424 = t427;
}
else
{
const t428 = f_std$_starts_at(v_s_417, v_i_420, "true");
let t429;
if (t428)
{
const t430 = $d("std.JBool", [true]);
$at = "463 行 53 列";
const t431 = $add(v_i_420, 4n);
const t432 = $d("std.Parsed{}", [true, t430, t431]);
t429 = t432;
}
else
{
const t433 = f_std$_starts_at(v_s_417, v_i_420, "false");
let t434;
if (t433)
{
const t435 = $d("std.JBool", [false]);
$at = "465 行 54 列";
const t436 = $add(v_i_420, 5n);
const t437 = $d("std.Parsed{}", [true, t435, t436]);
t434 = t437;
}
else
{
const t438 = $eq(v_c_422, "\"");
let t439;
if (t438)
{
const t440 = f_std$_parse_string(v_s_417, v_i_420);
t439 = t440;
}
else
{
const t441 = $eq(v_c_422, "[");
let t442;
if (t441)
{
$at = "469 行 24 列";
const t443 = $add(v_i_420, 1n);
const t444 = f_std$_parse_array(v_s_417, t443);
t442 = t444;
}
else
{
const t445 = $eq(v_c_422, "{");
let t446;
if (t445)
{
$at = "471 行 25 列";
const t447 = $add(v_i_420, 1n);
const t448 = f_std$_parse_object(v_s_417, t447);
t446 = t448;
}
else
{
let t449;
{
const t450 = f_std$_parse_int(v_s_417, v_i_420);
t449 = t450;
}
t446 = t449;
}
t442 = t446;
}
t439 = t442;
}
t434 = t439;
}
t429 = t434;
}
t424 = t429;
}
return t424;
};
const f_std$_parse_array = function(v_s_451, v_i_452) {
const t453 = [];
let v_items_454 = t453;
const t455 = f_std$_skip_ws(v_s_451, v_i_452);
let v_j_456 = t455;
const t457 = f_std$_char_at(v_s_451, $sh(v_j_456));
const t458 = $eq(t457, "]");
let v_done_459 = t458;
let t460;
if ($sh(v_done_459))
{
$at = "482 行 19 列";
const t461 = $add($sh(v_j_456), 1n);
v_j_456 = t461;
t460 = undefined;
}
let v_bad_462 = false;
while (true) {
const t463 = !$sh(v_done_459);
let t464 = t463;
if (t464)
{
const t465 = !$sh(v_bad_462);
t464 = t465;
}
if (!t464) break;
const t466 = f_std$_parse_value(v_s_451, $sh(v_j_456));
let v_p_467 = t466;
const t468 = $field(v_p_467, "ok");
const t469 = !t468;
let t470;
if (t469)
{
v_bad_462 = true;
t470 = undefined;
}
else
{
let t471;
{
const t472 = $field(v_p_467, "value");
v_items_454 = $opush(v_items_454, t472);
const t473 = $field(v_p_467, "pos");
const t474 = f_std$_skip_ws(v_s_451, t473);
v_j_456 = t474;
const t475 = f_std$_char_at(v_s_451, $sh(v_j_456));
let v_c_476 = t475;
const t477 = $eq(v_c_476, ",");
let t478;
if (t477)
{
$at = "493 行 21 列";
const t479 = $add($sh(v_j_456), 1n);
v_j_456 = t479;
t478 = undefined;
}
else
{
const t480 = $eq(v_c_476, "]");
let t481;
if (t480)
{
v_done_459 = true;
$at = "496 行 21 列";
const t482 = $add($sh(v_j_456), 1n);
v_j_456 = t482;
t481 = undefined;
}
else
{
let t483;
{
v_bad_462 = true;
t483 = undefined;
}
t481 = t483;
}
t478 = t481;
}
t471 = t478;
}
t470 = t471;
}
}
let t484;
if ($sh(v_bad_462))
{
const t485 = f_std$_no_parse();
t484 = t485;
}
else
{
let t486;
{
const t487 = $d("std.JArr", [$sh(v_items_454)]);
const t488 = $d("std.Parsed{}", [true, t487, $sh(v_j_456)]);
t486 = t488;
}
t484 = t486;
}
return t484;
};
const f_std$_parse_object = function(v_s_489, v_i_490) {
const t491 = [];
let v_fields_492 = t491;
const t493 = f_std$_skip_ws(v_s_489, v_i_490);
let v_j_494 = t493;
const t495 = f_std$_char_at(v_s_489, $sh(v_j_494));
const t496 = $eq(t495, "}");
let v_done_497 = t496;
let t498;
if ($sh(v_done_497))
{
$at = "510 行 19 列";
const t499 = $add($sh(v_j_494), 1n);
v_j_494 = t499;
t498 = undefined;
}
let v_bad_500 = false;
while (true) {
const t501 = !$sh(v_done_497);
let t502 = t501;
if (t502)
{
const t503 = !$sh(v_bad_500);
t502 = t503;
}
if (!t502) break;
const t504 = f_std$_skip_ws(v_s_489, $sh(v_j_494));
const t505 = f_std$_parse_string(v_s_489, t504);
let v_k_506 = t505;
const t507 = $field(v_k_506, "pos");
const t508 = f_std$_skip_ws(v_s_489, t507);
let v_colon_509 = t508;
const t510 = f_std$_skip_ws(v_s_489, $sh(v_j_494));
const t511 = f_std$_char_at(v_s_489, t510);
const t512 = !$eq(t511, "\"");
let t513 = t512;
if (!t513)
{
const t514 = $field(v_k_506, "ok");
const t515 = !t514;
t513 = t515;
}
let t516 = t513;
if (!t516)
{
const t517 = f_std$_char_at(v_s_489, v_colon_509);
const t518 = !$eq(t517, ":");
t516 = t518;
}
let t519;
if (t516)
{
v_bad_500 = true;
t519 = undefined;
}
else
{
let t520;
{
$at = "518 行 36 列";
const t521 = $add(v_colon_509, 1n);
const t522 = f_std$_parse_value(v_s_489, t521);
let v_v_523 = t522;
const t524 = $field(v_v_523, "ok");
const t525 = !t524;
let t526;
if (t525)
{
v_bad_500 = true;
t526 = undefined;
}
else
{
let t527;
{
const t528 = $field(v_k_506, "value");
let t529;
switch (t528.$c) {
case "std.JStr": {
const v_key_530 = t528.$f[0];
let t531;
{
const t532 = $field(v_v_523, "value");
const t533 = $d("std.Field{}", [v_key_530, t532]);
v_fields_492 = $opush(v_fields_492, t533);
t531 = undefined;
}
t529 = t531;
break;
}
default: {
let t534;
{
v_bad_500 = true;
t534 = undefined;
}
t529 = t534;
break;
}
}
const t535 = $field(v_v_523, "pos");
const t536 = f_std$_skip_ws(v_s_489, t535);
v_j_494 = t536;
const t537 = f_std$_char_at(v_s_489, $sh(v_j_494));
let v_c_538 = t537;
const t539 = $eq(v_c_538, ",");
let t540;
if (t539)
{
$at = "529 行 25 列";
const t541 = $add($sh(v_j_494), 1n);
v_j_494 = t541;
t540 = undefined;
}
else
{
const t542 = $eq(v_c_538, "}");
let t543;
if (t542)
{
v_done_497 = true;
$at = "532 行 25 列";
const t544 = $add($sh(v_j_494), 1n);
v_j_494 = t544;
t543 = undefined;
}
else
{
let t545;
{
v_bad_500 = true;
t545 = undefined;
}
t543 = t545;
}
t540 = t543;
}
t527 = t540;
}
t526 = t527;
}
t520 = t526;
}
t519 = t520;
}
}
let t546;
if ($sh(v_bad_500))
{
const t547 = f_std$_no_parse();
t546 = t547;
}
else
{
let t548;
{
const t549 = $d("std.JObj", [$sh(v_fields_492)]);
const t550 = $d("std.Parsed{}", [true, t549, $sh(v_j_494)]);
t548 = t550;
}
t546 = t548;
}
return t546;
};
const f_std$_parse_json = function(v_s_551) {
const t552 = f_std$_parse_value(v_s_551, 0n);
let v_p_553 = t552;
const t554 = $field(v_p_553, "ok");
let t555 = t554;
if (t555)
{
const t556 = $field(v_p_553, "pos");
const t557 = f_std$_skip_ws(v_s_551, t556);
const t558 = $len(v_s_551);
const t559 = $eq(t557, t558);
t555 = t559;
}
let t560;
if (t555)
{
const t561 = $field(v_p_553, "value");
const t562 = $d("std.Some", [t561]);
t560 = t562;
}
else
{
let t563;
{
const t564 = $d("std.None", []);
t563 = t564;
}
t560 = t563;
}
return t560;
};
const f_std$_field = function(v_j_565, v_key_566) {
const t567 = $d("std.None", []);
let v_r_568 = t567;
let t569;
switch (v_j_565.$c) {
case "std.JObj": {
const v_fs_570 = v_j_565.$f[0];
let t571;
{
for (const t572 of v_fs_570) {
const v_f_573 = t572;
const t574 = $field(v_f_573, "key");
const t575 = $eq(t574, v_key_566);
let t576;
if (t575)
{
const t577 = $field(v_f_573, "value");
const t578 = $d("std.Some", [t577]);
v_r_568 = t578;
t576 = undefined;
}
}
t571 = undefined;
}
t569 = t571;
break;
}
default: {
let t579;
{
t579 = undefined;
}
t569 = t579;
break;
}
}
return $sh(v_r_568);
};
const f_std$_str_or = function(v_j_580, v_d_581) {
let t582;
switch (v_j_580.$c) {
case "std.Some": {
const v_v_583 = v_j_580.$f[0];
let t584;
switch (v_v_583.$c) {
case "std.JStr": {
const v_s_585 = v_v_583.$f[0];
t584 = v_s_585;
break;
}
default: {
t584 = v_d_581;
break;
}
}
t582 = t584;
break;
}
case "std.None": {
t582 = v_d_581;
break;
}
}
return t582;
};
const f_std$_int_or = function(v_j_586, v_d_587) {
let t588;
switch (v_j_586.$c) {
case "std.Some": {
const v_v_589 = v_j_586.$f[0];
let t590;
switch (v_v_589.$c) {
case "std.JInt": {
const v_n_591 = v_v_589.$f[0];
t590 = v_n_591;
break;
}
default: {
t590 = v_d_587;
break;
}
}
t588 = t590;
break;
}
case "std.None": {
t588 = v_d_587;
break;
}
}
return t588;
};
const f_util$_add = function(a0, a1) { return $from("int", $imp_util$_add($to("int", a0), $to("int", a1))); };
const f_util$_shout = function(a0) { return $from("any", $imp_util$_shout($to("any", a0))); };
const f_util$_applyTwice = function(a0, a1) { return $from("int", $imp_util$_applyTwice($to({ fn: ["int"], ret: "int", async: false }, a0), $to("int", a1))); };
const f_util$_point = function(a0, a1) { return $from({ st: "util.Point{}", f: [["x", "int"], ["y", "int"]] }, $imp_util$_point($to("int", a0), $to("int", a1))); };
const f_util$_words = function(a0) { return $from({ vec: "any" }, $imp_util$_words($to("any", a0))); };
const f_util$_later = async function(a0) { return $from("int", await $imp_util$_later($to("int", a0))); };
const f_util$_mapLater = async function(a0, a1) { return $from({ vec: "int" }, await $imp_util$_mapLater($to({ vec: "int" }, a0), $to({ fn: ["int"], ret: "int", async: true }, a1))); };
const f_greet$_hello = function(a0) { return $from("any", $imp_greet$_hello($to("any", a0))); };
const f_greet$_size = function(a0) { return $from({ st: "greet.Size{}", f: [["w", "int"], ["h", "int"]] }, $imp_greet$_size($to({ st: "greet.Size{}", f: [["w", "int"], ["h", "int"]] }, a0))); };
const f_std$_Eq$impl0$eq = function(v_a_592, v_b_593) {
const t594 = $eq(v_a_592, v_b_593);
return t594;
};
const f_std$_Eq$impl0 = function() {
const t598 = (() => function(v_a_595, v_b_596) {
const t597 = f_std$_Eq$impl0$eq(v_a_595, v_b_596);
return t597;
})();
const t599 = $d("std.Eq{}", [t598]);
return t599;
};
const f_std$_Eq$impl1$eq = function(v_a_600, v_b_601) {
const t602 = $eq(v_a_600, v_b_601);
return t602;
};
const f_std$_Eq$impl1 = function() {
const t606 = (() => function(v_a_603, v_b_604) {
const t605 = f_std$_Eq$impl1$eq(v_a_603, v_b_604);
return t605;
})();
const t607 = $d("std.Eq{}", [t606]);
return t607;
};
const f_std$_Eq$impl2$eq = function(v_a_608, v_b_609) {
const t610 = $eq(v_a_608, v_b_609);
return t610;
};
const f_std$_Eq$impl2 = function() {
const t614 = (() => function(v_a_611, v_b_612) {
const t613 = f_std$_Eq$impl2$eq(v_a_611, v_b_612);
return t613;
})();
const t615 = $d("std.Eq{}", [t614]);
return t615;
};
const f_std$_Ord$impl3$lt = function(v_a_616, v_b_617) {
const t618 = (v_a_616 < v_b_617);
return t618;
};
const f_std$_Ord$impl3 = function() {
const t619 = f_std$_Eq$impl0();
const t620 = f_std$_Eq$get(undefined, t619);
const t624 = (() => function(v_a_621, v_b_622) {
const t623 = f_std$_Ord$impl3$lt(v_a_621, v_b_622);
return t623;
})();
const t625 = $d("std.Ord{}", [t620, t624]);
return t625;
};
const f_std$_Show$impl4$show = function(v_x_626) {
const t627 = $str(v_x_626);
return t627;
};
const f_std$_Show$impl4 = function() {
const t630 = (() => function(v_x_628) {
const t629 = f_std$_Show$impl4$show(v_x_628);
return t629;
})();
const t631 = $d("std.Show{}", [t630]);
return t631;
};
const f_std$_Show$impl5$show = function(v_x_632) {
return v_x_632;
};
const f_std$_Show$impl5 = function() {
const t635 = (() => function(v_x_633) {
const t634 = f_std$_Show$impl5$show(v_x_633);
return t634;
})();
const t636 = $d("std.Show{}", [t635]);
return t636;
};
const f_std$_Show$impl6$show = function(v_x_637) {
const t638 = $str(v_x_637);
return t638;
};
const f_std$_Show$impl6 = function() {
const t641 = (() => function(v_x_639) {
const t640 = f_std$_Show$impl6$show(v_x_639);
return t640;
})();
const t642 = $d("std.Show{}", [t641]);
return t642;
};
const f_std$_Eq$impl7$eq = function(v_A_643, v_a_644, v_b_645, v_inst$0_646) {
const t647 = $len(v_a_644);
const t648 = $len(v_b_645);
const t649 = !$eq(t647, t648);
let t650;
if (t649)
{
return false;
t650 = undefined;
}
const t651 = $len(v_a_644);
for (let t652 = 0n; t652 < t651; t652++) {
const v_i_653 = t652;
const t654 = $len(v_b_645);
const t655 = (v_i_653 < t654);
let t656;
if (t655)
{
const t657 = v_a_644[Number(v_i_653)];
const t658 = v_b_645[Number(v_i_653)];
const t659 = f_std$_eq(undefined, t657, t658, v_inst$0_646);
const t660 = !t659;
let t661;
if (t660)
{
return false;
t661 = undefined;
}
t656 = t661;
}
}
return true;
};
const f_std$_Eq$impl7 = function(v_A_662, v_inst$0_663) {
const t667 = (() => function(v_a_664, v_b_665) {
const t666 = f_std$_Eq$impl7$eq(undefined, v_a_664, v_b_665, v_inst$0_663);
return t666;
})();
const t668 = $d("std.Eq{}", [t667]);
return t668;
};
const f_std$_Show$impl8$show = function(v_A_669, v_xs_670, v_inst$0_671) {
let v_s_672 = "[";
const t673 = $len(v_xs_670);
for (let t674 = 0n; t674 < t673; t674++) {
const v_i_675 = t674;
const t676 = (v_i_675 > 0n);
let t677;
if (t676)
{
$at = "130 行 28 列";
const t678 = $add($sh(v_s_672), ", ");
v_s_672 = t678;
t677 = undefined;
}
$at = "131 行 17 列";
const t679 = v_xs_670[Number(v_i_675)];
const t680 = f_std$_show(undefined, t679, v_inst$0_671);
const t681 = $add($sh(v_s_672), t680);
v_s_672 = t681;
}
$at = "133 行 9 列";
const t682 = $add($sh(v_s_672), "]");
return t682;
};
const f_std$_Show$impl8 = function(v_A_683, v_inst$0_684) {
const t687 = (() => function(v_x_685) {
const t686 = f_std$_Show$impl8$show(undefined, v_x_685, v_inst$0_684);
return t686;
})();
const t688 = $d("std.Show{}", [t687]);
return t688;
};
(async () => { try { await f_main(); } catch (e) { console.error("実行時エラー: " + e.message + ($at ? "（" + $at + "）" : "")); if (globalThis.process) process.exitCode = 1; } })();
