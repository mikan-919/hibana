# 検証記録

2026-11-16、Linux x86-64 の Nix 開発環境で実行した。対象は今回の是正実装で、下記は終了コード0を確認したコマンドの結果。

| コマンド | 結果 |
|---|---|
| `nix develop -c cargo build` | 成功 |
| `nix develop -c target/debug/hibana hanabi/demo/build.hb -- "$PWD"` | 成功。`target/hanabi/` に実行ファイル、WASM、接続コード、配信ファイルを生成 |
| `nix develop -c cargo test` | 179件成功、0件失敗。ライブラリ97件、統合82件 |
| `nix develop -c target/debug/hibana test hanabi/tests/contracts.hb` | 4件中4件成功 |
| `cd hanabi && nix develop -c npm run test:browser` | `2 passed (5.7s)`。1280×800 と390×844 |
| `cd hanabi && nix develop -c npm run test:quality` | 6項目が pass。suspect は0件 |
| `git diff --check` | 指摘なし |

## 完了条件との対応

- [x] フレームワークへのデモ識別子の混入を `framework_sources_do_not_embed_demo_contracts_or_handlers` で検査。JavaScript は入力・描画・汎用イベント・通信・参照の寿命を扱い、投稿の型・要素名・保存先・経路はデモに置いた。
- [x] JSX は引用符付き属性、式、本文、自己終了タグ、型付きイベントを展開する。`api` の一つの宣言から入力・出力・エラー・経路引数を共有し、`routes` と `client.call` がその型を使う。
- [x] `Port`・`Status`・`Byte`・`RequestId`・添字範囲を型検査する。切り出しと連結の結果の長さも証明に使える。不正な定数、範囲、呼び出しの入力型、ハンドラーの出力型、能力と効果の不足を拒否するテストが通過。
- [x] ファイル不在は `None`、空ファイルは `Some([])`。HTTP テストで登録資産の不在404、存在する空ファイルの200、バイナリの内容を確認。
- [x] 全 hanabi の `.hb` と追加したランタイム・検証入力を整形済み。既存の整形テストに `hanabi/` を追加。ブラウザーテストで、空一覧・入力エラー・投稿一覧の状態と初期HTMLを通じ、CSSクラスの未使用と定義漏れが0件であることを確認。
- [x] ビルドは `tests/hanabi.rs` がデモのビルドスクリプトを実行して確認。実バイナリを別プロセスで起動し、HTTP の作成201・経路引数付き更新200・一覧200・不正入力422・欠落404・破損した保存内容500・切断後の継続・再起動後の保存を確認。
- [x] Playwright が一時ディレクトリへビルドし、空きポートで実バイナリを起動・終了・再起動する。フォームから空タイトル・121文字のタイトルを送信し、送信要求を型違反のJSONに変える場合も含め、HTTP422と画面のエラーを確認。作成・編集・一覧変化・文字列の表示・リロード・再起動後の保存を確認。
- [x] `hibana test` で `MemoryStorage` の差し替え、`Transport.exchange` の応答による再開、`Dom` と `Schedule` の差し替え、型付きセッションの更新と二重応答の無視、JSX と生成した変換を確認。
- [x] `tests/programs/macros/hanabi_contract.hb` を、既存のインタプリタ・JavaScript・WASM・ネイティブの出力比較で実行。`examples/first.hb` と正本のファイルに変更なし。
- [x] WASM の参照は100回の更新後も状態を保持し、別型・別インスタンス・数値による偽造・破棄済みの参照を拒否するテストが通過。

## 実行出力の抜粋

```text
cargo test:
  test result: ok. 97 passed; 0 failed
  tests/hanabi.rs: test result: ok. 5 passed; 0 failed
  他の統合テスト77件も成功。合計179件

hibana test:
  ok   保存先の差し替えと細分化型
  ok   通信効果の応答で再開する
  ok   型付きセッションと通信完了
  ok   JSX と生成した変換
  4 件中 4 件が成功

Playwright:
  desktop: ビルドしたサーバーで入力拒否・作成・編集・再起動後の保存を確認する
  mobile:  ビルドしたサーバーで入力拒否・作成・編集・再起動後の保存を確認する
  2 passed (5.7s)
```

画面検査は vlmkit 0.23.0、Node.js v24.21.0 を使用。integrity、copy、scroll、handlers、interactions、breakpoints が pass。handlers に、登録したイベントをこの一覧検査だけでは発火しないという警告が1件。interactions は警告0件で、実際のクリックとフォーム送信は Playwright でも実行した。

## 残る制約

サーバーは一つのプロセスで逐次処理する。要求ごとの fork と wait は使わない。言語のヒープには自動回収がなく、サーバーと持続するWASMセッションはメモリを消費する。WASM は dispose で領域を再利用できる。URLの百分率復号、認証、HTTPS、チャンク転送、並行書き込みは未対応。実装と制約は `hanabi/README.md` に記載した。
