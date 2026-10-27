// セルフホストのコンパイラーが使うファイルの入出力（JS ターゲットでだけ動く）
import { readFileSync } from "node:fs";
import { resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";

export function readFile(path: string): string {
  return readFileSync(path, "utf8");
}

export function args(): string[] {
  return process.argv.slice(2);
}

// 絶対パス
export function resolve(path: string): string {
  return resolvePath(path);
}

// lib/std.hb の絶対パス
export function stdPath(): string {
  return fileURLToPath(new URL("../lib/std.hb", import.meta.url));
}
