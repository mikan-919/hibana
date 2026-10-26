// 型宣言 greet.d.ts を隣に置いた JavaScript のモジュール
export function hello(name) {
  return "Hello, " + name;
}

export function size(o) {
  return { w: o.w * 2, h: o.h * 2 };
}
