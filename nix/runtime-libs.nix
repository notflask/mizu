# Libraries that winit / wgpu load with dlopen at run time.
{ lib, stdenv, wayland, libxkbcommon, vulkan-loader, libGL, libx11, libxcursor, libxi, libxrandr, libxcb }:
lib.optionals stdenv.hostPlatform.isLinux [
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
