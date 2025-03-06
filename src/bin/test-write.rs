use std::io::Write;

struct Writer;

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        dbg!(&buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        println!("flush");
        Ok(())
    }
}

fn main() {
    let mut writer = Writer;
    write!(writer, "{}", -20).unwrap();
}
