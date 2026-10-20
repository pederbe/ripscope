-- Ripscope's Neovim commands, ready without any setup. See docs/editors/neovim.md.
if vim.g.loaded_ripscope_nvim then return end
vim.g.loaded_ripscope_nvim = true
require('ripscope').commands()
