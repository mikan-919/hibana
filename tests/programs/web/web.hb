// web ライブラリ（lib/web.hb）を使う例。use web の名前は web.ok のように修飾して使う
// （use web -> * なら修飾なし、use web -> {ok, Request} なら挙げた名前だけ修飾なし）
use web
use std
use std -> { assert_eq }

fn durable_objects(name: str) -> str { 'stub(' + name + ')' }

fn main() {
    let user = 'Alice'
    let page = <div class={'card'}>
        Hello {user}!
        <Icon name={'star'}></Icon>
    </div>
    print(page)

    let app = routes {
        GET '/' {
            web.html(<h1>Welcome</h1>)
        }

        GET '/user/{id}' {
            web.ok('user ' + id)
        }

        @durable_objects('ACCOUNT') -> account
        POST '/post/{user}/{post}' {
            web.ok(account + ': post ' + post + ' by ' + user)
        }
    }
    for path in ['/', '/user/42', '/nothing'] {
        let res = app(web.request('GET', path))
        print(to_str(res.status) + ' ' + res.body)
    }
    print(app(web.request('POST', '/post/bob/7')))
}

// 既存コンストラクタはヘッダーなし。追加しても本文と状態を保つ
test 'response headers' {
    assert_eq(len(web.ok('ok').headers), 0)
    assert_eq(len(web.html('html').headers), 0)
    assert_eq(len(web.json('{}').headers), 0)
    assert_eq(len(web.status(201, 'created').headers), 0)
    assert_eq(len(web.not_found().headers), 0)
    let original = web.status(201, 'created')
    let first = web.with_header(original, 'Set-Cookie', 'session=one; HttpOnly')
    let res = web.with_header(first, 'Set-Cookie', 'theme=dark')
    assert_eq(res.status, 201)
    assert_eq(res.content_type, 'text/plain')
    assert_eq(res.body, 'created')
    assert_eq(len(original.headers), 0)
    assert_eq(len(first.headers), 1)
    assert_eq(len(res.headers), 2)
    if len(res.headers) == 2 {
        assert_eq(res.headers[0].name, 'Set-Cookie')
        assert_eq(res.headers[0].value, 'session=one; HttpOnly')
        assert_eq(res.headers[1].name, 'Set-Cookie')
        assert_eq(res.headers[1].value, 'theme=dark')
    } else { panic('ヘッダーの数が違う') }
}

test 'redirect after save' {
    let app = routes {
        POST '/posts' { web.redirect('/posts/42') }
    }
    let res = app(web.request('POST', '/posts'))
    assert_eq(res.status, 303)
    assert_eq(res.body, '')
    assert_eq(len(res.headers), 1)
    if len(res.headers) > 0 {
        assert_eq(res.headers[0].name, 'Location')
        assert_eq(res.headers[0].value, '/posts/42')
    }
}

test 'HTML text and attribute escaping' {
    assert_eq(web.escape_html('<&>"\'日本語😀'), '&lt;&amp;&gt;&quot;&#39;日本語😀')
    assert_eq(web.escape_html('&lt;'), '&amp;lt;')
}

test 'strict form decoding' {
    assert_eq(web.form_value('title=%E6%97%A5%E6%9C%AC%E8%AA%9E+%F0%9F%98%80&body=a%26b%3Dc%2B', 'title'), std.Some('日本語 😀'))
    assert_eq(web.form_value('title=x&body=a%26b%3Dc%2B', 'body'), std.Some('a&b=c+'))
    assert_eq(web.form_value('a=b=c&empty=&flag', 'a'), std.Some('b=c'))
    assert_eq(web.form_value('a=b=c&empty=&flag', 'empty'), std.Some(''))
    assert_eq(web.form_value('flag', 'flag'), std.Some(''))
    assert_eq(web.form_value('', 'absent'), std.None)
    assert_eq(web.form_valid(''), true)
    assert_eq(web.form_value('日本=生の文字', '日本'), std.Some('生の文字'))
    assert_eq(web.form_value('a=%00%7F%C2%80%DF%BF%E0%A0%80%EF%BF%BF%F0%90%80%80%F4%8F%BF%BF', 'a'), std.Some(char_from_codepoint(0) + char_from_codepoint(127) + char_from_codepoint(128) + char_from_codepoint(2047) + char_from_codepoint(2048) + char_from_codepoint(65535) + char_from_codepoint(65536) + char_from_codepoint(1114111)))
    for body in ['a=%', 'a=%0', 'a=%gg', 'a=%80', 'a=%C0%AF', 'a=%C1%BF', 'a=%E0%80%80', 'a=%ED%A0%80', 'a=%F0%80%80%80', 'a=%F4%90%80%80', 'a=%F5%80%80%80', 'a=%FF', 'a=%E6%97', 'a=%C2x', '%=ok', 'a=1&a=2', 'a=1&%61=2', 'a=ok&b=%GG'] {
        assert_eq(web.form_valid(body), false)
        assert_eq(web.form_value(body, 'a'), std.None)
    }
}

test 'unicode scalar construction' {
    assert_eq(char_from_codepoint(65), 'A')
    assert_eq(char_from_codepoint(26085), '日')
    assert_eq(char_from_codepoint(128512), '😀')
    assert_eq(char_from_codepoint(-1), '')
    assert_eq(char_from_codepoint(55296), '')
    assert_eq(char_from_codepoint(57343), '')
    assert_eq(char_from_codepoint(1114112), '')
    assert_eq(char_from_codepoint(9223372036854775807), '')
}
