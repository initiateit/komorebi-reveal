fn main() {
    let mut m: windows::Win32::Graphics::GdiPlus::ColorMatrix = unsafe { std::mem::zeroed() };
    m.m[0] = 1.0;
}
