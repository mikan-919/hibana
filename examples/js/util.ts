// hibana の extern から読み込む TypeScript のモジュール
export function add(a: number, b: number): number {
  return a + b;
}

export function shout(s: string): string {
  return s.toUpperCase() + "!";
}

export function applyTwice(f: (x: number) => number, x: number): number {
  return f(f(x));
}

export function point(x: number, y: number): { x: number; y: number } {
  return { x, y };
}
