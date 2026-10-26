// hibana から use "./js/util.ts" で読み込む TypeScript のモジュール
export interface Point {
  x: number;
  y: number;
}

export function add(a: number, b: number): number {
  return a + b;
}

export function shout(s: string): string {
  return `${s.toUpperCase()}!`;
}

export function applyTwice(f: (x: number) => number, x: number): number {
  return f(f(x));
}

export function point(x: number, y: number): Point {
  return { x, y };
}

export function words(s: string): string[] {
  return s.split(" ");
}
