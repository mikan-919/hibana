use std -> *

test 'Title' {
    describe 'Situation' {
        let db = dbMock()
        it 'calculate sum' {
            assert_eq(db.sql('...'), 5)
        }
    }
}
