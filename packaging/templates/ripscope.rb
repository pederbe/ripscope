class Ripscope < Formula
  desc "C navigation with atomic index updates"
  homepage "https://ripscope.com"
  url "https://github.com/pederbe/ripscope/releases/download/v@VERSION@/ripscope-@VERSION@-x86_64-linux-gnu.tar.gz"
  sha256 "@ARCHIVE_SHA256@"
  license all_of: [
    { any_of: ["MIT", "Apache-2.0"] },
    "GPL-2.0-or-later",
  ]

  depends_on "patchelf" => :build
  depends_on arch: :x86_64
  depends_on "gcc" # The archive imports libgcc_s.so.1.
  depends_on :linux

  # Keep the archive's required licences at their manifested paths.
  skip_clean "libexec"

  def install
    system "sha256sum", "-c", "MANIFEST.sha256"
    payload = libexec/"ripscope"
    payload.install "bin", "contrib", "licenses", "BUILD.json", "DEPENDENCIES.json",
                    "INSTALL.md", "LICENSE-MIT", "LICENSE-APACHE", "MANIFEST.sha256", "README.md"
    system Formula["patchelf"].opt_bin/"patchelf", "--set-rpath",
           (Formula["gcc"].opt_lib/"gcc/current").to_s, payload/"bin/ripscope"
    cd payload do
      mv "MANIFEST.sha256", "ORIGINAL-ARCHIVE-MANIFEST.sha256"
      (payload/"HOMEBREW-RELOCATION.txt").write <<~TEXT
        Homebrew set the GCC runtime path. Original archive SHA-256:
        @ARCHIVE_SHA256@
      TEXT
      system "sh", "-c", "set -eu; sha256sum bin/ripscope >> HOMEBREW-RELOCATION.txt; " \
                         "find . -type f ! -name MANIFEST.sha256 -print0 | " \
                         "sort -z | xargs -0 sha256sum > MANIFEST.sha256"
    end
    bin.install_symlink payload/"bin/ripscope"
    prefix.install_symlink payload/"LICENSE-MIT", payload/"LICENSE-APACHE", payload/"README.md"
  end

  test do
    (testpath/"check-installed.sh").write <<~'SH'
@CHECK_SCRIPT_RUBY@
    SH
    system "sh", testpath/"check-installed.sh", version.to_s, bin/"ripscope", libexec/"ripscope"
  end
end
