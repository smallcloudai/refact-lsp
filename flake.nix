{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs, ... }:
    let
      pkgs = import nixpkgs { system = "x86_64-linux"; };
    in
    {
      packages."x86_64-linux".refact-lsp =
        let
          src = builtins.path {
            name = "refact-lsp";
            path = ./.;
            filter = file: type: !builtins.elem (baseNameOf file) [
              "flake.nix"
              "flake.lock"
            ];
          };
        in
        pkgs.rust.packages.stable.rustPlatform.buildRustPackage {
          name = "refact-lsp";
          src = src;

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

      packages."x86_64-linux".refact = pkgs.python3Packages.buildPythonPackage {
        pname = "refact";
        version = "0.9.9";

        src = ./python_binding_and_cmdline;

        nativeBuildInputs = [
          self.packages."x86_64-linux".refact-lsp
        ];

        postPatch = ''
          mkdir -p ./refact/bin
          cp ${self.packages."x86_64-linux".refact-lsp}/bin/refact-lsp ./refact/bin/refact-lsp
        '';

        propagatedBuildInputs = with pkgs.python3Packages; [
          aiohttp
          termcolor
          pydantic
          prompt-toolkit
          requests
          pyyaml
          tabulate
          pyperclip
          rich
        ];
      };

      packages."x86_64-linux".default = self.packages."x86_64-linux".refact-lsp;

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
