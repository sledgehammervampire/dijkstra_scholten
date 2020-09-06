let
  sources = import ./nix/sources.nix;
  nixpkgs-mozilla = import sources.nixpkgs-mozilla;
  pkgs = import sources.nixpkgs {
    overlays = [
      nixpkgs-mozilla
      (self: super: {
        rustc = self.latest.rustChannels.stable.rust;
        # don't use cargo from overlay since broken
      })
    ];
  };
  naersk = pkgs.callPackage sources.naersk { };
  rust = pkgs.latest.rustChannels.stable.rust;
  dijkstraScholten = naersk.buildPackage {
    root = ./.;
    buildInputs = with pkgs; [ pkgconfig openssl ];
  };
in pkgs.mkShell {
  buildInputs = [ dijkstraScholten rust ]
    ++ (with pkgs; [ cargo-edit pkgconfig openssl ]);
}
