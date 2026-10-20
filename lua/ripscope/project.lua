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

function M.same(left, right)
  left, right = M.clean(left), M.clean(right)
  if windows then return left:lower() == right:lower() end
  return left == right
end

-- Where the installer puts the executable.
function M.installer_executable()
  local home = windows and vim.env.USERPROFILE or vim.env.HOME
  local directory = vim.env.RIPSCOPE_INSTALL_DIR or ((home or '') .. '/.local/bin')
  return M.clean(directory) .. (windows and '/ripscope.exe' or '/ripscope')
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
-- { root, index, selection, published }.
function M.locate(exec, path, cwd)
  local result = vim.system({ exec, '--project', path }, { cwd = cwd, text = true }):wait()
  if result.code ~= 0 then
    error((result.stderr or ''):gsub('^ripscope: ', ''):gsub('\n+$', ''), 0)
  end
  local found = vim.json.decode(result.stdout)
  assert(type(found) == 'table' and type(found.root) == 'string' and type(found.index) == 'string'
    and type(found.selection) == 'table', 'unexpected ripscope --project report')
  return found
end

-- The version of the Ripscope repository this plugin came from, or nil.
function M.checkout_version(repository)
  local manifest = io.open(repository .. '/Cargo.toml', 'r')
  if not manifest then return nil end
  for line in manifest:lines() do
    local version = line:match('^version = "([^"]+)"')
    if version then
      manifest:close()
      return version
    end
  end
  manifest:close()
  return nil
end

-- Whether version OLD is older than NEW, by their numbers.
function M.older(old, new)
  local function numbers(version)
    local list = {}
    for number in (version:match('^[0-9.]*') or ''):gmatch('%d+') do list[#list + 1] = tonumber(number) end
    return list
  end
  local a, b = numbers(old), numbers(new)
  for position = 1, math.max(#a, #b) do
    local x, y = a[position] or 0, b[position] or 0
    if x ~= y then return x < y end
  end
  return false
end

return M
