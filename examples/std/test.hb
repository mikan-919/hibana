use std -> *

// データベースの代わり。sql はどの問い合わせにも決まった値を返す
struct DbMock { sum: int }

fn dbMock() -> DbMock { DbMock { sum: 5 } }

fn DbMock::sql(db: DbMock, query: str) -> int { db.sum }

test 'Title' {
    describe 'Situation' {
        let db = dbMock()
        it 'calculate sum' {
            assert_eq(db.sql('...'), 5)
        }
    }
}
