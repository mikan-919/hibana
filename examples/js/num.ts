// selfhost が読める型（string・number・boolean・void とその配列）だけの TypeScript モジュール
export function add(a: number, b: number): number {
  return a + b;
}

export function sum(xs: number[]): number {
  return xs.reduce((s, x) => s + x, 0);
}

export function range(n: number): number[] {
  return Array.from({ length: n }, (_, i) => i);
}
