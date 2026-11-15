fn main() {
    write_bytes('dist/app.mjs', compile_file('main.hb', 'js', []))
}
