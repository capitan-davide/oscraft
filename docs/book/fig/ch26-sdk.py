from svg import Fig
f = Fig(800, 400)
# the SDK
f.box("g", 20, 16, 240, 368); f.text(140, 38, "oscraft-sdk-<version>-aarch64", "b c")
items = [("sysroot/", "headers, libraries, crt objects, rtld"), ("clang.cfg", "target, sysroot, lld"), ("bin/oscraft-cc, oscraft-c++", "the host's clang with clang.cfg"), ("oscraft.cmake", "CMake toolchain file"), ("cargo-config.toml", "target, build-std, the linker"), ("MANIFEST", "OsCraft and FreeBSD commits")]
for i, (a, b) in enumerate(items):
    y = 52 + i * 54
    f.box("w", 34, y, 212, 44, 2); f.text(140, y + 19, a, "ms c"); f.text(140, y + 35, b, "s c")
# C path
f.box("y", 300, 52, 130, 44, 3); f.text(365, 71, "hello.c", "m c"); f.text(365, 87, "CMakeLists.txt", "ms c")
f.box("w", 470, 52, 150, 44, 3); f.text(545, 71, "cmake", "b c"); f.text(545, 87, "with oscraft.cmake", "s c")
f.box("u", 660, 52, 120, 44, 3); f.text(720, 71, "sdk-c", "m c"); f.text(720, 87, "dynamic", "s c")
f.arrow([(430, 74), (468, 74)]); f.arrow([(620, 74), (658, 74)])
# Rust path
f.box("y", 300, 136, 130, 44, 3); f.text(365, 155, "main.rs", "m c"); f.text(365, 171, "Cargo.toml", "ms c")
f.box("w", 470, 136, 150, 56, 3); f.text(545, 155, "cargo +nightly", "b c"); f.text(545, 171, "builds std from source,", "s c"); f.text(545, 184, "links with oscraft-cc", "s c")
f.box("u", 660, 136, 120, 44, 3); f.text(720, 155, "sdk-rust", "m c"); f.text(720, 171, "static", "s c")
f.arrow([(430, 158), (468, 158)]); f.arrow([(620, 158), (658, 158)])
f.arrow([(260, 114), (545, 114), (545, 98)], "lng", "ag"); f.arrow([(260, 210), (545, 210), (545, 194)], "lng", "ag"); f.text(280, 228, "the SDK configures the host's tools", "s")
f.text(540, 36, "any directory, the host's own clang, lld, CMake and Cargo", "s c")
# into an image
f.box("y", 420, 236, 360, 44, 3); f.text(600, 255, "/proc/boot/sdk-c=<path>   /proc/boot/sdk-rust=<path>", "ms c"); f.text(600, 271, "two lines appended to the build file (IMAGE_EXTRA)", "s c")
f.arrow([(780, 74), (792, 74), (792, 258), (782, 258)]); f.arrow([(720, 180), (720, 234)])
f.box("k", 420, 312, 170, 44, 3); f.text(505, 331, "make image", "m c"); f.text(505, 347, "build/oscraft.ifs", "ms c")
f.box("n", 630, 312, 150, 44, 3); f.text(705, 331, "QEMU", "b c"); f.text(705, 347, "run both, check the output", "s c")
f.arrow([(505, 280), (505, 310)]); f.arrow([(590, 334), (628, 334)])
f.save("ch26-sdk.svg")
