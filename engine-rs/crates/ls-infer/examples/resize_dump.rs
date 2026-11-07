//! 诊断工具：对输入 BGR raw 文件做 cv2 复刻 INTER_LINEAR 缩放，输出 uint8 raw。
//! 用法：cargo run -p ls-infer --example resize_dump -- <src.raw> <sw> <sh> <dw> <dh> <out.raw>
//! （src.raw = sw*sh*3 字节 BGR；out.raw = dw*dh*3 字节 BGR，供 Python cv2 逐像素 diff）

use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err("用法: resize_dump <src.raw> <sw> <sh> <dw> <dh> <out.raw>".into());
    }
    let src = std::fs::read(&args[0])?;
    let sw: u32 = args[1].parse()?;
    let sh: u32 = args[2].parse()?;
    let dw: u32 = args[3].parse()?;
    let dh: u32 = args[4].parse()?;
    let expect = (sw as usize) * (sh as usize) * 3;
    if src.len() < expect {
        return Err(format!("输入尺寸不符: {} < {expect}", src.len()).into());
    }
    let dst = ls_infer::yolo::resize_u8_linear_cv2(&src, sw, sh, dw, dh, 3);
    let mut f = std::fs::File::create(&args[5])?;
    f.write_all(&dst)?;
    println!("OK {} bytes", dst.len());
    Ok(())
}
