# Libraries needed to build and run mizu on Linux: fontconfig/freetype are
# linked, the rest is loaded with dlopen by winit / wgpu.
{ lib, stdenv, fontconfig, freetype, wayland, libxkbcommon, vulkan-loader, libGL, libx11, libxcursor, libxi, libxrandr, libxcb }:
lib.optionals stdenv.hostPlatform.isLinux [
  # mupdf's `system-fonts` feature (font-kit) links these.
  fontconfig
  freetype
  wayland
  libxkbcommon
  vulkan-loader
  libGL
  libx11
  libxcursor
  libxi
  libxrandr
  libxcb
]
