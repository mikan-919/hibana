# hanabi

hanabi は、hibana でサーバーと画面を書く Web フルスタックフレームワークです。サーバーは Linux x86-64 のネイティブ実行ファイル、画面は WebAssembly（WASM。ブラウザーで実行する形式）にコンパイルします。JavaScript は DOM（画面の要素）と通信を WASM に接続します。

## 投稿アプリを動かす

リポジトリのルートで実行します。

```sh
nix develop -c cargo build
nix develop -c target/debug/hibana hanabi/build.hb
chmod +x target/hanabi/hanabi
cd target/hanabi
./hanabi
```

`http://127.0.0.1:8787` を開きます。投稿の作成・編集・一覧表示ができ、投稿は作業ディレクトリの `data/posts.json` に保存します。再起動後も同じ作業ディレクトリで起動してください。再ビルドする前にサーバーを終了してください。

## 入力条件を型で共有する

[demo/domain.hb](demo/domain.hb) は、画面とサーバーが共有する投稿の契約です。細分化型（条件を満たす値だけを受け入れる型）を使います。

```hb
type PostId = int[id -> id > 0]
type Title = String[s -> 1 <= len(s) && len(s) <= 120]
type Body = String[s -> 1 <= len(s) && len(s) <= 4000]
struct Draft { title: Title, body: Body }
```

`draft(title, body)` は空白を除き、`refine<Title>` と `refine<Body>` で実行時に条件を検査します。成功した値は `Title`・`Body` として使えます。画面とサーバーの両方がこの関数を呼びます。長さは Unicode の符号位置の数です。

## 保存先を with で渡す

`Posts` はトレイト（型が提供する操作の契約）です。投稿操作は `with Posts(store)` で保存先を要求します。

```hb
fn create_post(input: Draft) -> Post ! Failure
with Posts(store) { store.create(input) }
```

[demo/stores.hb](demo/stores.hb) に、JSON ファイルを使う `FilePosts` と、参照を使う `MemoryPosts` があります。アプリは `with Posts(FilePosts { path: 'data/posts.json' })` で実装を選びます。テストでは同じ操作に `MemoryPosts` を渡します。保存先の指定が無い呼び出しは型検査で拒否します。

## 経路と失敗を宣言する

[demo/server.hb](demo/server.hb) は次の形で経路を書きます。`routes` はマクロ（構文を展開する機能）で、メソッドと経路の重複をコンパイル時に拒否します。

```hb
fn endpoints(request: Request) -> Response ! Rejected, Failure
with Posts(store) {
    routes request {
        GET '/api/posts' -> { ok(list_posts()) }
        POST '/api/posts' -> { created(create_post(json_body<Draft>(request))) }
        PUT '/api/posts' -> {
            let input = json_body<Edit>(request)
            ok(update_post(input.id, input.draft))
        }
    }
}
```

`json_body<T>` は `Decode` の実装で JSON を `T` に変換します。`ok` と `created` は `ToJson` の実装で応答を作ります。[demo/codec.hb](demo/codec.hb) の `object { ... }` もマクロで、フィールドの値を `ToJson` で変換します。

入力の拒否 `Rejected` と投稿操作の失敗 `Failure` は、代数的効果（処理を呼び出し側に委ねる仕組み）です。`app` の `handle` が入力エラーを HTTP 422、投稿の欠落を 404、保存内容のエラーを 500 に変換します。処理しない効果は型検査で拒否します。

## 画面と通信を hibana で書く

[demo/client.hb](demo/client.hb) は `view` マクロで画面を作ります。

```hb
view article(class = 'post') {
    view h3 { text(post.title) };
    view p(class = 'post-body') { text(post.body) }
}
```

子要素は `Html` 型です。`text` は `<` などを文字として表示する形に変換し、文字列を子要素に渡すと型検査で拒否します。属性の値も変換します。`Html` を直接構築する場合と、属性に URL やイベント処理を指定する場合の検査は呼び出し側の責任です。

`query` と `send<T: ToJson>` は `Remote` 効果を起こします。`client.run` が通信要求を JavaScript に渡し、応答時に WASM の `receive` を呼びます。現在の接続は通信を一件ずつ行い、応答はコールバックで処理します。効果の継続を通信完了まで保持する仕組みはありません。

## 検証

```sh
# リポジトリのルート
nix develop -c cargo test --test hanabi --test hanabi_native

# 上の手順でサーバーを起動してから、別の端末で実行
cd hanabi
nix develop -c npm ci
nix develop -c npx playwright install chromium
nix develop -c npm run test:browser
```

Rust のテストは保存先と効果の不足、型の違反、経路の重複、ネイティブサーバーの作成・編集・再起動後の読み込みを検証します。ブラウザーテストはデスクトップ幅とスマートフォン幅で入力拒否・作成・編集・再読み込み・文字の表示を確認します。ブラウザーテストは起動中のサーバーに投稿を追加します。

## 実行範囲

現在のサーバーは `127.0.0.1` で待ち受け、要求ごとにプロセスを作って順に処理します。HTTP ヘッダーは 16KiB、本文は 128KiB、保存ファイルは 1MiB 未満です。HTTP のチャンク転送は受け付けません。保存は一時ファイルを書いて同期してから置き換えます。ファイルやソケットのシステム呼び出しが失敗すると、そのプロセスは終了します。

認証、HTTPS、要求の時間制限、データベース、複数プロセスからの保存は未実装です。ネイティブの通信・ファイル操作を WASM に含めるとコンパイルエラーになります。
