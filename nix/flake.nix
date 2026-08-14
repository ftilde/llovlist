{
  description = "Android SDK/NDK for building the 1kauf app";

  # Pinned to a commit, not a branch: later 25.11 revisions make build-tools pull an
  # i686-linux dependency that cannot build without 32-bit support.
  inputs.nixpkgs.url =
    "github:NixOS/nixpkgs/e576e3c9cf9bad747afcddd9e34f51d18c855b4e";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        config = {
          android_sdk.accept_license = true;
          allowUnfree = true;
        };
      };
      # dx's gradle template pins compileSdk 34, and AGP then demands exactly these.
      android = pkgs.androidenv.composeAndroidPackages {
        platformVersions = [ "34" ];
        buildToolsVersions = [ "34.0.0" ];
        includeNDK = true;
        includeEmulator = false;
        includeSystemImages = false;
      };
      sdk = "${android.androidsdk}/libexec/android-sdk";
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = [ android.androidsdk pkgs.jdk17 ];

        # dx reads ANDROID_SDK_ROOT before ANDROID_HOME; both are set so neither can win.
        ANDROID_HOME = sdk;
        ANDROID_SDK_ROOT = sdk;
        JAVA_HOME = pkgs.jdk17.home;

        # AGP fetches its own aapt2 from maven, which cannot run on NixOS-style paths.
        GRADLE_OPTS =
          "-Dorg.gradle.project.android.aapt2FromMavenOverride=${sdk}/build-tools/34.0.0/aapt2";

        # No adb here: a second adb of a different version fights the running server.
        shellHook = ''
          export ANDROID_NDK_HOME="$(echo ${sdk}/ndk/* | cut -d' ' -f1)"
          export NDK_HOME="$ANDROID_NDK_HOME"
        '';
      };
    };
}
