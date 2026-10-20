# Uninstalling Ripscope

## Installed with a script

The install script removes what it installed: the executable, its notices
directories and the editor integrations. On Linux, run

```sh
curl -fsSL https://ripscope.com/install.sh | sh -s -- --uninstall
```

In PowerShell, run

```powershell
& ([scriptblock]::Create((irm https://ripscope.com/install.ps1))) -Uninstall
```

Set `RIPSCOPE_INSTALL_DIR` as you did when installing if you chose another
directory. Remove the line you added to your Emacs init file yourself. On Windows,
close the editors running Ripscope first: Windows keeps an executable in use, so the
uninstaller says so, removes everything else, and removes the executable when run
again.

The indexes the editors made stay in your cache directory, `~/.cache/ripscope` on
Linux and `%LOCALAPPDATA%\ripscope` on Windows, or the folder `RIPSCOPE_CACHE_DIR`
names when you set it. Delete them too if you no longer need them. Indexes you built
in your projects stay there.

## Installed with a plugin manager

Remove the plugin's line from your editor configuration and let the plugin manager
clean up, for example with `:PlugClean` or lazy.nvim's `:Lazy clean`, or
`M-x package-delete` in Emacs. Then run the uninstall command above to remove the
executable the plugin installed.

## Installed with a system package

On Debian or Ubuntu, run

```sh
sudo apt remove ripscope
```

On Fedora or RHEL-family systems, run

```sh
sudo dnf remove ripscope
```

## Installed from an archive or source

For an archive installation, remove its extracted directory after closing any
editor that uses it. For `cargo install`, run `cargo uninstall ripscope`.
