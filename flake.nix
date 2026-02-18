{
  description = "Statespace - open-source AI runtime";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    crane.url = "github:ipetkov/crane";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, fenix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ fenix.overlays.default ];
        };

        inherit (pkgs.stdenv) isDarwin isLinux;

        # Stable Rust toolchain via fenix (consistent with gateway)
        rustToolchain = pkgs.fenix.stable.withComponents [
          "cargo"
          "clippy"
          "rust-src"
          "rust-analyzer"
          "rustc"
          "rustfmt"
        ];

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Filter source to only Rust-relevant files -- avoids cache busting
        # on docs/site/scripts changes
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (craneLib.filterCargoSources path type) ||
            (builtins.match ".*\\.toml$" path != null) ||
            (builtins.match ".*\\.lock$" path != null);
        };

        # Build inputs -- statespace has no heavy C deps:
        # - libiconv on Darwin (reqwest/rustls pulls it in via core-foundation-sys)
        # - nothing on Linux (rustls-tls means no OpenSSL)
        buildInputs = pkgs.lib.optionals isDarwin [
          pkgs.libiconv
        ];

        workspaceVersion = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

        commonArgs = {
          inherit src;
          pname = "statespace-workspace";
          version = workspaceVersion;
          strictDeps = true;
          inherit buildInputs;
          nativeBuildInputs = [ rustToolchain ];

          # Skip tests in Nix builds -- run separately via cargo in test.yml
          doCheck = false;
        };

        # Stage 1: build all workspace deps as a separate derivation.
        # Nix/FlakeHub caches this independently so subsequent builds only
        # recompile changed crates.
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Stage 2: build the statespace binary
        statespace = craneLib.buildPackage (commonArgs // {
          pname = "statespace";
          inherit cargoArtifacts;
          cargoExtraArgs = "--package statespace";
        });

        # --- musl static builds (Linux only) ---
        # We build musl natively on the matching arch runner (x86_64 or aarch64),
        # so there is no cross-compilation -- just a different libc.
        # pkgsCross gives us the musl stdenv + linker while keeping the same arch.
        muslPkgs =
          if isLinux then
            (if pkgs.stdenv.hostPlatform.isAarch64
             then pkgs.pkgsCross.aarch64-multiplatform-musl
             else pkgs.pkgsCross.musl64)
          else null;

        muslCraneLib =
          if isLinux then
            # overrideToolchain must take a function (p: ...) when cross-compiling
            # so crane can splice it correctly against the musl pkgs instance
            ((crane.mkLib muslPkgs).overrideToolchain (_muslPkgs:
              pkgs.fenix.combine [
                pkgs.fenix.stable.cargo
                pkgs.fenix.stable.rustc
                (if pkgs.stdenv.hostPlatform.isAarch64
                 then pkgs.fenix.targets.aarch64-unknown-linux-musl.stable.rust-std
                 else pkgs.fenix.targets.x86_64-unknown-linux-musl.stable.rust-std)
              ]
            ))
          else null;

        muslTarget =
          if isLinux then
            (if pkgs.stdenv.hostPlatform.isAarch64
             then "aarch64-unknown-linux-musl"
             else "x86_64-unknown-linux-musl")
          else null;

        muslCommonArgs =
          if isLinux then {
            inherit src;
            pname = "statespace-workspace-musl";
            version = workspaceVersion;
            strictDeps = true;
            doCheck = false;

            # No buildInputs needed -- musl is self-contained
            CARGO_BUILD_TARGET = muslTarget;
            # Produce a fully static binary
            CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
          } else null;

        muslCargoArtifacts =
          if isLinux then muslCraneLib.buildDepsOnly muslCommonArgs else null;

        statespace-musl =
          if isLinux then
            muslCraneLib.buildPackage (muslCommonArgs // {
              pname = "statespace-musl";
              cargoArtifacts = muslCargoArtifacts;
              cargoExtraArgs = "--package statespace";
            })
          else null;

      in
      {
        packages = {
          default = statespace;
          inherit statespace;
        } // pkgs.lib.optionalAttrs isLinux {
          inherit statespace-musl;
        };

        checks = {
          statespace-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--package statespace -- --deny warnings";
          });

          fmt = craneLib.cargoFmt { inherit src; pname = "statespace-workspace"; };
        };

        # Dev shell -- system deps only, Rust from rustup (same pattern as gateway)
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            pkg-config
          ] ++ pkgs.lib.optionals isDarwin [
            libiconv
          ];
        };
      }
    );
}
