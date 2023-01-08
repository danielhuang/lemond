fn main() {
    cc::Build::new()
        .file("src/procmon/procmon.c")
        .include("src")
        .compile("procmon");
}
