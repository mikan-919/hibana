# hanabi

hibana でサーバーと画面を書くフレームワークです。サーバーは Linux x86-64 のネイティブ実行ファイル、画面は WebAssembly（WASM。ブラウザーの実行形式）にコンパイルします。

## ビルドと起動

リポジトリのルートで実行します。

```sh
nix develop -c cargo build
nix develop -c target/debug/hibana hanabi/demo/build.hb -- "$PWD"
chmod +x target/hanabi/application
cd target/hanabi
./application
```

`http://127.0.0.1:8787` を開きます。投稿は起動ディレクトリの `data/posts.json` に保存します。ビルドの第2引数は出力ディレクトリの絶対経路、第3引数はポート番号です。テストは空きポートと一時ディレクトリを使います。

```sh
nix develop -c target/debug/hibana hanabi/demo/build.hb -- "$PWD" /tmp/hanabi-app 18080
```

[build.hb](build.hb) は `Build` で指定したサービス、入口関数、ポート、クライアント、公開関数、配信ファイルを生成します。デモの指定は [demo/build.hb](demo/build.hb) にあります。静的ファイルは [demo/service.hb](demo/service.hb) の `Config.assets` に登録します。登録したファイルが不在なら 404、空ファイルが存在すれば 200 と空の本文を返します。

## 共有エンドポイント

[demo/api.hb](demo/api.hb) が、メソッド・URL・経路引数・入力・出力・エラー・成功コードの正本です。

```hb
api {
    list_api GET '/api/posts' (unit, unit) -> Vec<Post> ! Fault = 200
    create_api POST '/api/posts' (unit, Form) -> Post ! Fault = 201
    update_api PUT '/api/posts/{id}' (PostId, Form) -> Post ! Fault = 200
}
```

`api` は宣言マクロ（型や関数を生成する構文拡張）で、各定義から `Endpoint<P, I, O, E>` を返す関数を生成します。重複したメソッドと経路の形、不正な経路引数、成功コードの範囲違反はコンパイル時に拒否します。経路引数が一つならスカラー、複数なら `record` を使います。経路の文字列から、引数の型に従って復号します。

サーバーは同じ定義へハンドラーを接続します。

```hb
routes request {
    list_api -> fn(_, _) { stores.list(repo) };
    create_api -> fn(_, form) { stores.create(repo, form) };
    update_api -> fn(id, form) { stores.update(repo, id, form) }
}
```

引数と結果の型が定義に合わなければ型エラーです。`routes` が入力の復号、経路引数の検査、成功応答、エラー応答を処理します。アプリに `handle` の入れ子はありません。`Fault` は `HttpError` と `Codec` を実装し、別のエラー型もこの二つを実装して使えます。

クライアントは `client.call(update_api(), id, form, fn(result) { Saved(result) })` と書きます。URL・メソッドの文字列をクライアントに再記述しません。返るメッセージの中身は `Result<Post, Fault>` です。

## 型と変換の生成

[demo/domain.hb](demo/domain.hb) で型とフィールドを宣言します。

```hb
scalar PostId(int[n -> n > 0])
scalar Title(String[s -> 1 <= len(s) && len(s) <= 120])
scalar Body(String[s -> 1 <= len(s) && len(s) <= 4000])
record Form { title: String, body: String }
record Draft { title: Title, body: Body }
record Post { id: PostId, title: Title, body: Body }
```

`scalar` は細分化型（条件を満たす値だけを許す型）を持つ別型と `Codec` の実装を生成します。`Title('')` と `PostId(0)` はコンパイル時に拒否します。復号時は同じ条件を検査します。`record` は構造体、JSON の符号化・復号、経路引数の復号を生成します。アプリにフィールド名を繰り返す変換関数はありません。

フォームの通信型は `Form`、検証後の業務入力は `Draft` です。サーバーが `Draft` を得るまで保存操作へ進みません。空入力や長さ違反もブラウザーから送信し、HTTP 422 の応答を画面へ戻します。文字数は Unicode の符号位置の数です。

基盤の [types.hb](types.hb) には `Port`、`Status`、`Byte`、`Bytes`、`Socket` があります。[native.hb](native.hb) の `slice` は、前の引数を参照する細分化条件で `0 <= start <= end <= len(bytes)` を要求します。呼び出し側で証明できない値は渡せません。

## JSX とイベント

[view.hb](view.hb) は JSX（タグと式を組み合わせる構文）を展開します。

```hb
<article class="post">
    <h3>{post.title.0}</h3>
    <button type="button" disabled={model.busy} onClick={Edit(post.id)}>編集</button>
</article>
```

`type`・`for` と、`aria-label` などの属性を書けます。自己終了タグも使えます。本文と属性値を HTML 用に変換し、閉じタグの不一致を拒否します。`disabled={false}` は属性を出しません。`onClick`・`onSubmit`・`onChange` は、`union Event { ... }` から生成した変換でメッセージを配送します。イベント名・投稿 ID の数値化・アプリの要素名を JavaScript に書きません。

`Html` の直接構築と、URL を受け取る属性の安全性は呼び出し側で管理します。経路引数の生成では `/`・`?`・`#`・`%` を拒否します。URL の百分率復号は未対応です。

## 状態・通信・保存の境界

`Program<S, M>` が初期状態、更新関数、表示関数、イベント復号を持ちます。更新は `Change<S, M>` を返し、通信は `Command<M>` として返します。`Fetch` に要求と結果を受け取る関数を保持し、応答ごとに型付きメッセージを生成します。文字列フラグによる応答の振り分けはありません。

`Session<S, M>` を `Ref`（値を保持する参照）で WASM 内に保持します。JavaScript には生成コードが管理する参照の取っ手を渡します。モデルや投稿を JavaScript の `Map` に保存しません。別の型・インスタンス・破棄済みの取っ手は生成コードで拒否します。

`Transport.exchange` は応答を返す効果（処理をハンドラーへ委ねる操作）です。`client.perform` のテストでは `resume(reply)` で結果を差し替えます。ブラウザーは `Schedule` 効果をコマンドの予約に使い、通信完了時に `client.resolve` へ戻します。`Dom` 効果もテストで値の読み取り・描画・フォーカスを差し替えられます。

`Storage` は JSON の保存・読み出し、バイナリ資産の読み出し、保存先の作成を表す能力です。`with Storage(...)` を要求し、`NativeStorage` と `MemoryStorage` で同じアプリの操作を実行します。ファイル不在と空ファイルは別の値です。書き込みは一時ファイルを同期してから置き換え、OS の読み書きの失敗は `Result` で返します。

## 検証

```sh
# リポジトリのルート
nix develop -c cargo build
nix develop -c cargo test
nix develop -c target/debug/hibana test hanabi/tests/contracts.hb

# ブラウザー。テストがビルド・起動・再起動・終了まで行う
cd hanabi
nix develop -c npm ci
nix develop -c npx playwright install chromium
nix develop -c npm run test:browser
nix develop -c npm run test:quality
```

Rust の統合テストは、ビルドスクリプトを実行して `target/hanabi/` を生成し、実バイナリとの HTTP 通信で作成・更新・一覧・不正入力・資産の不在・空ファイル・切断・再起動後の保存を確認します。ブラウザーテストはデスクトップ幅とスマートフォン幅で、フォーム送信による 422 とエラー表示、作成、編集、リロードとサーバー再起動後の保存を確認します。

## 実行範囲

サーバーは `127.0.0.1` で待ち受け、一つのプロセスで要求を順に処理します。ヘッダーは 16KiB、本文は 128KiB、保存ファイルは 1MiB 未満です。認証、HTTPS、チャンク転送、並行書き込みには対応していません。

hibana のヒープには自動回収がなく、継続して処理するサーバーと WASM セッションはメモリを消費します。WASM は `dispose()` でセッションの領域を再利用でき、破棄した参照は使えません。継続的な運用には言語のメモリ回収を実装する必要があります。
