{ lib, stdenvNoCC, fetchurl, autoPatchelfHook, gcc }:

stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "ripscope-bin";
  version = "@VERSION@";
  src = fetchurl {
    url = "https://github.com/pederbe/ripscope/releases/download/v${finalAttrs.version}/ripscope-${finalAttrs.version}-x86_64-linux-gnu.tar.gz";
    sha256 = "@ARCHIVE_SHA256@";
  };
  nativeBuildInputs = [ autoPatchelfHook ];
  buildInputs = [ gcc.cc.lib ];
  dontBuild = true;
  dontStrip = true;
  installPhase = ''
    runHook preInstall
    sha256sum -c MANIFEST.sha256
    mkdir -p "$out/lib/ripscope" "$out/bin"
    cp -a . "$out/lib/ripscope/"
    ln -s ../lib/ripscope/bin/ripscope "$out/bin/ripscope"
    runHook postInstall
  '';
  # autoPatchelfHook changes the interpreter and runtime search path.
  preFixup = ''
    recordRelocation() {
      (
        cd "$out/lib/ripscope"
        mv MANIFEST.sha256 ORIGINAL-ARCHIVE-MANIFEST.sha256
        printf 'Nix relocated the ELF interpreter/runtime paths. Original archive SHA-256: @ARCHIVE_SHA256@\n' > NIX-RELOCATION.txt
        sha256sum bin/ripscope >> NIX-RELOCATION.txt
        find . -type f ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 sha256sum > MANIFEST.sha256
      )
    }
    # Append after autoPatchelfHook, so the new manifest describes the final ELF.
    postFixupHooks+=(recordRelocation)
  '';
  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    sh ${./check-installed.sh} ${finalAttrs.version} "$out/bin/ripscope" "$out/lib/ripscope"
    runHook postInstallCheck
  '';
  meta = {
    description = "C navigation with atomic index updates";
    homepage = "https://ripscope.com";
    license = with lib.licenses; [ mit asl20 gpl2Plus ];
    platforms = [ "x86_64-linux" ];
    sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
    mainProgram = "ripscope";
  };
})
