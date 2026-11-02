-- Checks `ripscope lsp` through Neovim's built-in client on a fresh project, and exits
-- with the number of failed steps.
local root = vim.env.RIPSCOPE_LSP_PROJECT
local failures = 0

local function write(name, lines)
  local file = assert(io.open(root .. '/' .. name, 'wb'))
  file:write(table.concat(lines, '\n') .. '\n')
  file:close()
end

vim.fn.mkdir(root .. '/.git', 'p')
write('greet.h', { 'void greet(const char *name);', 'void shout(const char *text);' })
write('greet.c', {
  '#include "greet.h"', '', 'static int count;', '', 'void greet(const char *name)', '{',
  '\tcount++;', '\tshout(name);', '}',
})
write('main.c', { '#include "greet.h"', '', 'int main(void)', '{', '\tgreet("world");', '\treturn 0;', '}' })
write('shout.c', { 'void shout(const char *text)', '{', '}' })

local function check(step, found, expected)
  local same = vim.deep_equal(found, expected)
  if not same then failures = failures + 1 end
  print((same and 'ok   ' or 'FAIL ') .. step .. ': ' .. vim.inspect(found)
    .. (same and '' or ', expected ' .. vim.inspect(expected)))
end

local function settle(done) return vim.wait(15000, done, 20) end

local function file() return vim.fs.basename(vim.api.nvim_buf_get_name(0)) end

local function where()
  local cursor = vim.api.nvim_win_get_cursor(0)
  return file() .. ':' .. cursor[1] .. ':' .. cursor[2]
end

local function items(list)
  local found = {}
  for _, item in ipairs(list or {}) do
    local name = item.filename or vim.api.nvim_buf_get_name(item.bufnr)
    found[#found + 1] = vim.fs.basename(name) .. ':' .. item.lnum .. ':' .. item.col .. ':' .. item.text
  end
  table.sort(found)
  return found
end

local function at(name, line, column)
  vim.cmd.edit(vim.fn.fnameescape(root .. '/' .. name))
  vim.api.nvim_win_set_cursor(0, { line, column })
end

local function definition(target)
  vim.lsp.buf.definition()
  settle(function() return file() == target end)
  return where()
end

local function calls(ask)
  vim.fn.setqflist({})
  ask()
  settle(function() return #vim.fn.getqflist() > 0 end)
  local found = items(vim.fn.getqflist())
  vim.cmd.cclose()
  return found
end

local function symbols(query)
  local found
  vim.lsp.buf.workspace_symbol(query, { on_list = function(list) found = list.items end })
  settle(function() return found ~= nil end)
  return items(found)
end

vim.cmd.filetype('on')
vim.lsp.config('ripscope', {
  cmd = { vim.env.RIPSCOPE_LSP_BACKEND, 'lsp' },
  filetypes = { 'c' },
  root_markers = { 'cscope.files', '.git' },
})
vim.lsp.enable('ripscope')

at('main.c', 5, 1)
check('attached', settle(function()
  return #vim.lsp.get_clients({ bufnr = 0, name = 'ripscope' }) > 0
end), true)
check('definition', definition('greet.c'), 'greet.c:5:5')

at('main.c', 5, 1)
local references
vim.lsp.buf.references({ includeDeclaration = true }, {
  on_list = function(list) references = list.items end,
})
settle(function() return references ~= nil end)
check('references', items(references), {
  'greet.c:5:6:void greet(const char *name)',
  'greet.h:1:6:void greet(const char *name);',
  'main.c:5:2:\tgreet("world");',
})

at('main.c', 5, 1)
check('incoming calls', calls(vim.lsp.buf.incoming_calls), { 'main.c:5:2:main' })
-- Neovim lists outgoing calls in the current file, so ask on the definition.
at('greet.c', 5, 5)
check('outgoing calls', calls(vim.lsp.buf.outgoing_calls), { 'greet.c:8:2:shout' })

check('workspace symbols', symbols('gree'), { 'greet.c:5:6:[Function] greet' })

at('shout.c', 1, 0)
vim.api.nvim_buf_set_lines(0, -1, -1, false, { '', 'void whisper(void)', '{', '}' })
vim.cmd.write()
check('saved symbol', symbols('whisper'), { 'shout.c:5:6:[Function] whisper' })
at('main.c', 1, 0)
vim.api.nvim_buf_set_lines(0, 5, 5, false, { '\twhisper();' })
vim.cmd.write()
vim.api.nvim_win_set_cursor(0, { 6, 1 })
check('saved definition', definition('shout.c'), 'shout.c:5:5')

vim.cmd(failures == 0 and 'qall!' or ('cquit! ' .. failures))
