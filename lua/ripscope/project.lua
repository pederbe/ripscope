-- Finds the ripscope executable, and through it a file's project and index, which
-- `ripscope --project` reports, so that every editor finds the same ones.
local M = {}
local windows = vim.fn.has('win32') == 1

-- PATH with forward slashes and no trailing slash, except for a file system root.
function M.clean(path)
  path = vim.fs.normalize(path, { expand_env = false })
  if windows then path = path:gsub('\\', '/') end
  if path:match('^/$') or path:match('^%a:/$') then return path end
  return (path:gsub('/+$', ''))
end

-- The ripscope executable: on PATH, else where the installer puts it, else nil.
function M.executable()
  local found = vim.fn.exepath('ripscope')
  if found ~= '' and (not windows or found:lower():match('%.exe$')) then return M.clean(found) end
  local name = windows and '/ripscope.exe' or '/ripscope'
  local home = windows and vim.env.USERPROFILE or vim.env.HOME
  for _, directory in ipairs({ vim.env.RIPSCOPE_INSTALL_DIR or '', home and home .. '/.local/bin' or '' }) do
    if directory ~= '' and vim.fn.executable(M.clean(directory) .. name) == 1 then
      return M.clean(directory) .. name
    end
  end
  return nil
end

-- The project of PATH, a file or a directory, seen from the editor's directory CWD:
-- { root, index, selection, published }. OPTIONS, an index build's -f and selection
-- options, name another index and other sources to judge publication by.
function M.locate(exec, path, cwd, options)
  local command = { exec, '--project', path }
  vim.list_extend(command, options or {})
  local result = vim.system(command, { cwd = cwd, text = true }):wait()
  if result.code ~= 0 then
    error((result.stderr or ''):gsub('^ripscope: ', ''):gsub('\n+$', ''), 0)
  end
  local found = vim.json.decode(result.stdout)
  assert(type(found) == 'table' and type(found.root) == 'string' and type(found.index) == 'string'
    and type(found.selection) == 'table', 'unexpected ripscope --project report')
  return found
end

return M
