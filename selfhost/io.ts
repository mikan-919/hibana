// セルフホストのコンパイラーが使うファイルの入出力（JS ターゲットでだけ動く）
import { readFileSync } from "node:fs";

export function readFile(path: string): string {
  return readFileSync(path, "utf8");
}

export function args(): string[] {
  return process.argv.slice(2);
}
