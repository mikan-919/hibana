use std -> *
use std::math
use std::math -> { floor }

// ==========================================
// 1. 構文拡張 (syntax & macro)
// ==========================================

macro Unless('unless' expr:Expr block:Block) {
    quote { if !($expr) $block }
}

syntax Annotation = '@' ~ ident:Ident ~ '(' name:String ')'

macro TaggedBlock(ann:Annotation block:Block) {
    quote {
        put('[' + '${ann.ident}' + '] start: ' + ${ann.name})
        $block
        put('[' + '${ann.ident}' + '] end: ' + ${ann.name})
    }
}

// ==========================================
// 2. 細分化型 (type で部分型 / struct でNewtype)
// ==========================================

type Digit = int[x -> 0 <= x && x < 10]
type NonZero = int[x -> x != 0]
type Positive = number[x -> x > 0]

// 別型として厳密に区別したいものは struct で包む
struct UserId(int[x -> x > 0])

// ==========================================
// 3. データ型 (struct & enum)
// ==========================================

struct Point {
    x: number
    y: number
}

enum Shape {
    Circle(Point, Positive)
    Rect(Point, Point)
    Empty
}

// ==========================================
// 4. Trait と Capability (with)
// ==========================================

trait Display {
    fn to_string(self) -> String
}

impl Display for Point {
    fn to_string(self) -> String {
        '(' + self.x.to_string() + ', ' + self.y.to_string() + ')'
    }
}

// struct で包んだ型は中身のメソッドを引き継がないので、自分で付け直す
impl Display for UserId {
    fn to_string(self) -> String {
        self.0.to_string()
    }
}

trait Logger {
    fn log(self, msg: String)
}

struct SimpleLogger {}
impl Logger for SimpleLogger {
    fn log(self, msg: String) {
        put(msg)
    }
}

struct PrefixLogger {
    prefix: String
}
impl Logger for PrefixLogger {
    fn log(self, msg: String) {
        put(self.prefix + msg)
    }
}

// ==========================================
// 5. ロジック (Z3検証 & with による依存注入)
// ==========================================

// with なしの純粋関数。Z3によりゼロ除算が起きないことが静的に保証される
fn safe_div(a: int, b: NonZero) -> int {
    a / b
}

// Logger コンテキストを要求する関数
fn describe_shape(shape: Shape)
with Logger(logger) {
    match shape {
        Shape::Circle(center, r) -> {
            // Positive は number の部分型(type)なので .to_string() がそのまま呼べる
            logger.log('Circle at ' + center.to_string() + ', r=' + r.to_string())
        }
        Shape::Rect(p1, p2) -> {
            logger.log('Rect from ' + p1.to_string() + ' to ' + p2.to_string())
        }
        Shape::Empty -> {
            logger.log('Empty shape')
        }
    }
}

// ==========================================
// 6. Result (? で伝播 / match で剥く)
// ==========================================

// 実行時にしか分からない失敗は Result で返す
fn parse_digit(s: String) -> Result<Digit, String> {
    let n = parse_int(s)?
    if 0 <= n && n < 10 {
        // この中では n が Digit だと証明できる
        Ok(n)
    } else {
        Err('not a digit')
    }
}

// ==========================================
// 7. 非同期 (with Async / .await / join / spawn)
// ==========================================

// Async を要求する関数。呼んだだけでは走らず、Async<T> の値になる
// (実際は TypeScript の Promise を返す関数などを呼ぶ。ここでは値をそのまま返す)
fn fetch_name(id: UserId) -> String with Async {
    'user' + id.to_string()
}

fn fetch_score(id: UserId) -> int with Async {
    id.0 * 10
}

// 待つ場所に .await を書く。種類の違う 2 つは join で並行に走らせて合流する
fn profile(id: UserId) -> String with Async {
    let p = join(fetch_name(id), fetch_score(id)).await
    p.first + ': ' + p.second.to_string()
}

// Async を持たない関数から始めるには spawn を使う (色を止める)
// spawn は Async<T> を走らせ始め、要求を持たない Task<T> を返す
fn notify(id: UserId) {
    spawn(profile(id)).then(fn(s) { put('notified ' + s) })
}

// ==========================================
// 8. エントリポイント
// ==========================================

fn main() with Async {
    put('Hello, world!')

    // カスタム構文マクロ (@section)
    let x = 10
    let y = 20
    @section('math test') {
        put(math::sqrt(x + y**2).to_string())
        put(floor(x / y).to_string())
    }

    // マクロ展開 × Z3のパス依存検証
    let uid = UserId(1)
    put('uid = ' + uid.to_string())
    let d: Digit = 5
    unless d == 0 {
        // Unless展開後の !(d == 0) をZ3が仮定に積むため NonZero として通る
        put('100 / d = ' + safe_div(100, d).to_string())
    }

    // with スコープによる Trait 実装の差し替え
    let circle = Shape::Circle(Point { x: 0, y: 0 }, 10)

    with Logger(SimpleLogger {}) {
        describe_shape(circle)
    }

    with Logger(PrefixLogger { prefix: '[DEBUG] ' }) {
        describe_shape(Shape::Empty)
    }

    // 非同期: .await を付けない呼び出しは走らない値になる
    let later = profile(uid)
    put(later.await)
    // 無名関数で包めば、待たずに Async<T> の Vec を作れる。all で並行に走らせる
    let ids = [UserId(2), UserId(3)]
    put(all(map(ids, fn(id) { profile(id) })).await)

    // 標準入力 × パターンマッチ
    put('What color do you like?')
    let str = input()
    match str {
        'red'  -> { put('nice') }
        'blue' -> { put('cool') }
        _      -> { put('ok') }
    }

    // Result を match で剥く
    put('Pick a digit')
    match parse_digit(input()) {
        Ok(digit) -> { put('digit: ' + digit.to_string()) }
        Err(e)    -> { put('error: ' + e) }
    }

    // spawn した処理は、JavaScript では main が戻った後に終わる
    // 出力の順を実行方式で揃えるため、最後に置く
    notify(uid)
}
