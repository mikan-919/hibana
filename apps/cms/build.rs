// CMS の Hibana ソースを公開コンパイラー API で JavaScript にする。
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("apps/cms/main.hb"))?;
    let javascript = hibana::to_js(&source).map_err(|e| e.to_string())?;
    let out = root.join("apps/cms/dist");
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("app.mjs"), javascript)?;
    println!("生成: apps/cms/dist/app.mjs");
    Ok(())
}
