// hibana から extern で呼ぶ JavaScript のモジュール
export function hello(name) {
  return "Hello, " + name;
}

export function size(o) {
  return { w: o.w * 2, h: o.h * 2 };
}
