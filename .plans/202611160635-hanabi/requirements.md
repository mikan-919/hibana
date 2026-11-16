# Goal
hibana でサーバーと画面を書く Web フルスタックフレームワーク hanabi を作る。

# Req
R1 サーバーはネイティブ実行ファイル、画面は WASM と JavaScript で動かす。
R2 最初のアプリで投稿の作成・編集・一覧表示を行う。
R3 投稿は保存し、サーバーの再起動後も読み出せる。
R4 投稿の操作と入力検証を hibana で書き、ブラウザとサーバーの通信を通して実行する。
R5 ビルドでサーバーの実行ファイル、画面の WASM、JavaScript、静的ファイルを生成する。
R6 入力条件は細分化型を正本とし、サーバーと画面で同じ条件を検証する。
R7 経路と画面はマクロで記述し、経路の重複と HTML の子要素の型の誤りを検出する。
R8 投稿操作は with で要求する保存先に依存し、ファイル保存とメモリ保存で同じ関数を使う。
R9 通信要求と失敗を効果として表し、実行環境との接続とエラー応答は呼び出し側で処理する。

# Constraints
K1 既存の examples は変更せず、hanabi の実装と動作確認用アプリを別のディレクトリに置く。
K2 ネイティブ出力は現在の x86-64 Linux を対象にする。
K3 最初の保存先には JSON ファイルを使う。データベースへの接続は後の作業とする。

# Done
A1 [R1,R2,R4] ブラウザで投稿を作成・編集し、一覧の変化を確認できる。
A2 [R3] サーバーを再起動して同じ投稿を表示できる。
A3 [R4] 不正な入力をサーバーで拒否し、利用者に結果を表示する。
A4 [R5] リポジトリ内の手順でビルドと起動を再現できる。
A5 [R6,R7,R8,R9] 不正な型、重複経路、保存先の不足、未処理の効果を検査が拒否する。

# Verification
2026-11-16 に確認した。

- A1,A3: `cd hanabi && nix develop -c npm run test:browser`。1280px と 390px で入力拒否・作成・編集・再読み込み・HTML の文字表示を確認。
- A2: `nix develop -c cargo test --test hanabi_native`。サーバー再起動後の投稿と保存内容を照合。
- A4: `nix develop -c cargo build` と `nix develop -c target/debug/hibana hanabi/build.hb` で生成し、`target/hanabi` から起動。
- A5: `nix develop -c cargo test --test hanabi`。MemoryPosts の操作、型と効果の違反、経路の重複、ネイティブの接続契約と WASM の拒否を確認。
- 回帰: `nix develop -c cargo test -- --test-threads=1` が通過。正本 first.hb の4出力先の比較も含む。
- 画面: vlmkit 0.23.0 の integrity、copy、scroll、handlers、interactions、breakpoints が通過。interactions は入力クリアの反応を観測できない警告が1件。ブラウザーテストで Enter による値の消去・通知・入力欄への移動を確認する。
