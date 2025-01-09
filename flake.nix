{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs, ... }:
    let
      pkgs = import nixpkgs { system = "x86_64-linux"; };
    in
    {
      packages."x86_64-linux".default =
        pkgs.rust.packages.stable.rustPlatform.buildRustPackage {
          name = "refact-lsp";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
            protobuf
            rustfmt
          ];

          buildInputs = with pkgs; [
            openssl
          ];
        };

      devShells."x86_64-linux".default =
        (self.packages."x86_64-linux".default).overrideAttrs (self: super: {
          nativeBuildInputs = super.nativeBuildInputs ++ (with pkgs; [
            cargo
            clippy
            rust-analyzer
          ]);
        });

    };
}
