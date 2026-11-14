test 'Title' {
    describe 'Situation' {
        let db = dbMock()
        it 'calculate sum' {
            assert(db.sql('...').await, 5)
        }
    }
}
