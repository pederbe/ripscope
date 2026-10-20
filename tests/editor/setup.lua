-- One-step setup check, run by check-editor-setup in a fresh profile: open a C file,
-- search, save an edit and search again, with the plugin loaded as Neovim loads packages.
local report = { errors = {}, searches = {} }
local function path(name)
  return (vim.fs.normalize(vim.fn.fnamemodify(name, ':p')):gsub('\\', '/'))
end
local function search(pattern)
  vim.cmd('RipscopeFind g ' .. pattern)
  local ripscope = require('ripscope')
  assert(vim.wait(90000, function()
    local status = ripscope.status()
    return status ~= nil and status.state == 'complete'
  end, 20), 'search did not complete')
  local status = ripscope.status()
  local rows = {}
  for _, item in ipairs(vim.fn.getqflist({ id = status.quickfix_id, items = 1 }).items) do
    rows[#rows + 1] = { file = path(vim.api.nvim_buf_get_name(item.bufnr)), line = item.lnum, text = item.text }
  end
  local jump = vim.empty_dict()
  if #rows > 0 then
    vim.cmd('cfirst')
    jump = { file = path(vim.api.nvim_buf_get_name(0)), line = vim.api.nvim_win_get_cursor(0)[1],
      text = vim.api.nvim_get_current_line() }
  end
  report.searches[#report.searches + 1] = { pattern = pattern, rows = rows, jump = jump,
    ok = status.ok, message = status.message, session_pid = status.session_pid }
end
local ok, problem = xpcall(function()
  vim.cmd('edit ' .. vim.fn.fnameescape(vim.env.RIPSCOPE_SETUP_FILE))
  search('greet')
  -- From the results window, which shows no file: the same project, not Neovim's
  -- current directory.
  vim.cmd('copen')
  search('main')
  if (vim.env.RIPSCOPE_SETUP_ABSENT or '') ~= '' then search(vim.env.RIPSCOPE_SETUP_ABSENT) end
  vim.cmd('edit ' .. vim.fn.fnameescape(vim.env.RIPSCOPE_SETUP_FILE))
  vim.api.nvim_buf_set_lines(0, -1, -1, false, { 'int fresh_symbol(void) { return 3; }' })
  vim.cmd('write')
  search('fresh_symbol')
end, debug.traceback)
if not ok then report.errors[#report.errors + 1] = tostring(problem) end
report.messages = vim.fn.execute('messages')
local file = assert(io.open(vim.env.RIPSCOPE_SETUP_REPORT, 'w'))
file:write(vim.json.encode(report))
file:close()
vim.cmd('qall!')
