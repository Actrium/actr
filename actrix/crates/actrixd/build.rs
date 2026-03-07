fn main() {
    // Rebuild when admin UI source changes
    println!("cargo:rerun-if-changed=admin/web/src");
    println!("cargo:rerun-if-changed=admin/web/index.html");
    println!("cargo:rerun-if-changed=admin/web/package.json");

    #[cfg(feature = "admin-ui")]
    {
        let dist_index = std::path::Path::new("admin/web/dist/index.html");
        if !dist_index.exists() {
            println!(
                "cargo:warning=Admin UI dist not found. \
                 Run `cd crates/actrixd/admin/web && npm install && npm run build` first, \
                 or disable the admin-ui feature."
            );
        }
    }
}
