-- Ripscope's Neovim adapter. With no settings, a search finds the current file's
-- project and indexes it into your cache directory the first time. setup() with a
-- root chooses the project, index and sources yourself. See docs/editors/neovim.md.
local M = {}
local windows = vim.fn.has('win32') == 1
local project = require('ripscope.project')
-- The project setup() names with a root, or nil for automatic projects.
local explicit
-- Settings for automatic projects, from setup() without a root.
local defaults = {}
-- Each project's settings and its session, by root and index.
local projects = {}
-- What `ripscope --project` reported, by the directory searched from and Neovim's.
local located = {}
-- The automatic project last searched, for a search from a window that shows no file,
-- such as the quickfix list a search opens.
local recent
local running, last, log_buffer
local fields = { s = 0, g = 1, d = 2, c = 3, t = 4, e = 6, f = 7, i = 8, a = 9 }
local valid = { [0] = true, [1] = true, [2] = true, [3] = true, [4] = true,
  [6] = true, [7] = true, [8] = true, [9] = true }

local function string_arg(value, name)
  assert(type(value) == 'string' and value ~= '' and not value:find('%z'), name .. ' must be a nonempty NUL-free string')
  return value
end

local function absolute(path, root)
  local rooted = path:sub(1, 1) == '/'
  if windows then
    path = path:gsub('\\', '/')
    assert(not path:match('^//[?.]/'), 'device namespace paths are unsupported')
    if path:match('^%a:') then
      assert(path:sub(3, 3) == '/', 'use a full drive path, not a drive-relative path')
      rooted = true
    elseif path:sub(1, 2) == '//' then
      assert(path:match('^//[^/]+/[^/]+'), 'UNC paths require both server and share')
      rooted = true
    else
      assert(path:sub(1, 1) ~= '/', 'rooted Windows paths require an explicit drive or UNC share')
      rooted = false
    end
  end
  return vim.fs.normalize(rooted and path or (root .. '/' .. path), { expand_env = false })
end

local function log(text)
  if not log_buffer or not vim.api.nvim_buf_is_valid(log_buffer) then
    log_buffer = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_buf_set_name(log_buffer, 'ripscope://log')
    vim.bo[log_buffer].bufhidden = 'hide'
    vim.bo[log_buffer].swapfile = false
  end
  vim.bo[log_buffer].modifiable = true
  vim.api.nvim_buf_set_lines(log_buffer, 0, -1, false, vim.split(text, '\n', { plain = true }))
  vim.bo[log_buffer].modifiable = false
end

function M.show_log()
  if log_buffer and vim.api.nvim_buf_is_valid(log_buffer) then
    vim.cmd('botright sbuffer ' .. log_buffer)
  end
end

local function row_item(row, root)
  assert(type(row) == 'table' and vim.islist(row) and #row == 4, 'expected four-field JSON row')
  string_arg(row[1], 'result path')
  assert(type(row[2]) == 'string' and type(row[3]) == 'number'
    and row[3] > 0 and row[3] < math.huge and row[3] == math.floor(row[3]), 'invalid context or line')
  local text = row[4]
  if type(text) == 'table' and vim.islist(text) then
    local bytes = {}
    for _, byte in ipairs(text) do
      assert(type(byte) == 'number' and byte >= 0 and byte <= 255 and byte == math.floor(byte), 'invalid source byte')
      bytes[#bytes + 1] = string.format('\\x%02x', byte)
    end
    text = table.concat(bytes)
  end
  assert(type(text) == 'string', 'invalid source text')
  -- A text search keeps a CR LF line's carriage return, which the list need not show.
  text = text:gsub('\r$', '')
  return { filename = absolute(row[1], root), lnum = row[3],
    text = '<<' .. row[2] .. '>> ' .. text }
end

local function session_of(entry)
  if not entry.session then
    local opts = entry.config
    entry.session = require('ripscope.session').new(opts, function(row) return row_item(row, opts.root) end)
  end
  return entry.session
end

local function close_sessions()
  for _, entry in pairs(projects) do
    if entry.session then entry.session.close() end
  end
end

-- Runs one operation for ENTRY's project; AFTER runs once a successful one completes.
local function start(entry, kind, args, title, after)
  assert(not running, 'Ripscope operation is already running')
  local qfid
  if kind == 'query' then
    vim.fn.setqflist({}, ' ', { title = title .. ' (running)', items = {} })
    qfid = vim.fn.getqflist({ id = 0 }).id
  end
  local opts = entry.config
  log(title .. '\nRunning')
  running = true
  last = { kind = kind, state = 'running', quickfix_id = qfid }
  local function complete(result)
    local ok = result.code == 0 and result.signal == 0
    local message = kind == 'build' and 'Build complete' or 'Query complete'
    local parsed
    if ok and kind == 'query' then
      parsed = result.items
      message = #parsed == 0 and 'No matches' or ('Query complete: ' .. #parsed .. ' results')
    elseif not ok then
      message = result.message or string.format('%s failed (exit %s, signal %s)', kind, result.code, result.signal)
      if kind == 'build' then
        message = message .. '; completion unconfirmed, no freshness or rollback guarantee'
      end
    end
    local published, problem = pcall(function()
      if qfid then
        assert(vim.fn.setqflist({}, 'r', { id = qfid, title = title .. ': ' .. message,
          items = ok and parsed or {} }) == 0, 'quickfix list no longer exists')
      end
    end)
    if not published then ok, message = false, 'Result publication failed: ' .. tostring(problem) end
    -- A warning from ripscope, such as a file a build or a text search could not use.
    local note = ok and (result.stderr or ''):match('ripscope: ([^\n]+)')
    if note then message = message .. ' (' .. note .. ')' end
    last = { kind = kind, state = 'complete', ok = ok, message = message,
      code = result.code, signal = result.signal, stdout = result.stdout or '',
      stderr = result.stderr or '', quickfix_id = qfid, session_pid = result.session_pid }
    running = false
    log(title .. '\n' .. message .. '\n' .. last.stderr .. '\n' .. last.stdout)
    local level = not ok and vim.log.levels.ERROR or (note and vim.log.levels.WARN or vim.log.levels.INFO)
    vim.notify('Ripscope: ' .. message, level)
    if opts.open then
      if not ok then M.show_log()
      elseif qfid and #parsed > 0 and vim.fn.getqflist({ id = 0 }).id == qfid then vim.cmd.copen() end
    end
    if ok and after then after() end
  end
  local function completed(result)
    local ok, err = pcall(complete, result)
    if not ok then
      running = false
      last = { kind = kind, state = 'complete', ok = false, message = tostring(err), quickfix_id = qfid }
      vim.notify('Ripscope completion failed: ' .. tostring(err), vim.log.levels.ERROR)
    end
  end
  if kind == 'query' then
    local session = session_of(entry)
    local launched, problem = pcall(session.query, args, completed)
    if not launched then
      session.close()
      completed({ code = -1, signal = 0, stdout = '', stderr = tostring(problem) })
    end
    return
  end
  local launched, problem = pcall(vim.system, args, { cwd = opts.root, text = true, timeout = opts.timeout }, function(result)
    vim.schedule(function() completed(result) end)
  end)
  if not launched then
    complete({ code = -1, signal = 0, stdout = '', stderr = tostring(problem) })
  end
end

-- Checks the settings both kinds of project take, filling in their defaults.
local function settings(options)
  assert(vim.fn.has('nvim-0.10') == 1 and (windows or vim.uv.os_uname().sysname == 'Linux'),
    'requires Linux or native Windows Neovim 0.10 or newer')
  local opts = vim.deepcopy(options or {})
  local allowed = { exec = true, root = true, index = true, recursive = true,
    include_dirs = true, sources = true, file_list = true, timeout = true, open = true,
    auto_refresh = true }
  for key in pairs(opts) do assert(allowed[key], 'unknown Ripscope option: ' .. tostring(key)) end
  if opts.index then string_arg(opts.index, 'index') end
  if opts.exec then string_arg(opts.exec, 'exec') end
  opts.timeout = opts.timeout or 120000
  assert(type(opts.timeout) == 'number' and opts.timeout > 0 and opts.timeout < math.huge
    and opts.timeout == math.floor(opts.timeout), 'timeout must be positive integer milliseconds')
  if opts.open == nil then opts.open = true end
  if opts.auto_refresh == nil then opts.auto_refresh = true end
  for _, key in ipairs({ 'recursive', 'open', 'auto_refresh' }) do
    assert(opts[key] == nil or type(opts[key]) == 'boolean', key .. ' must be boolean')
  end
  for _, key in ipairs({ 'sources', 'include_dirs' }) do
    opts[key] = opts[key] or {}
    assert(type(opts[key]) == 'table' and vim.islist(opts[key]), key .. ' must be a list')
    for _, value in ipairs(opts[key]) do string_arg(value, key) end
  end
  if opts.file_list then
    string_arg(opts.file_list, 'file_list')
    assert(#opts.sources == 0, 'choose sources or file_list, not both')
  end
  return opts
end

local function executable(exec)
  local found = exec and vim.fn.exepath(exec) or project.executable() or ''
  assert(found ~= '', exec and 'Ripscope executable not found'
    or 'Ripscope executable not found; install it with :RipscopeInstall')
  if windows then
    assert(found:lower():match('%.exe$'), 'Windows backend must be a native .exe, not a shell script')
  end
  return absolute(found, vim.fn.getcwd())
end

-- The project the next operation is for: the one setup() named, or the current file's,
-- which `ripscope --project` finds.
local function current()
  if explicit then return explicit end
  local file = vim.api.nvim_buf_get_name(0)
  local is_file = file ~= '' and vim.bo.buftype == ''
  if not is_file and recent then return recent end
  local opts = settings(defaults)
  local cwd = vim.fn.getcwd()
  local path = is_file and file or cwd
  local exec = executable(opts.exec)
  local place = (path == cwd and cwd or vim.fs.dirname(path)) .. '\0' .. cwd
  local found = located[place]
  if not found then
    found = project.locate(exec, path, cwd)
    located[place] = found
  end
  opts.root = found.root
  -- Settings that choose the sources replace the selection the project has.
  local chosen = opts.recursive ~= nil or opts.file_list or #opts.sources > 0
  if not chosen then opts.selection = found.selection end
  local published = found.published
  if opts.index then
    opts.index = absolute(opts.index, found.root)
  else
    opts.index = found.index
  end
  if opts.index ~= found.index or chosen then
    -- `published` speaks for the cache index built with the project's own selection.
    published = vim.uv.fs_stat(opts.index) ~= nil
  end
  local key = opts.root .. '\0' .. opts.index
  local entry = projects[key]
  if not entry then
    opts.exec = exec
    if opts.selection then
      -- A session that finds no published index builds it as an index build would.
      opts.session_args = vim.deepcopy(opts.selection)
      for _, path in ipairs(opts.include_dirs) do
        vim.list_extend(opts.session_args, { '-I', path })
      end
    end
    entry = { config = opts, automatic = true, published = published }
    projects[key] = entry
  end
  recent = entry
  return entry
end

local function build_args(opts)
  if opts.selection then
    local args = { opts.exec, '-b' }
    vim.list_extend(args, opts.selection)
    for _, path in ipairs(opts.include_dirs) do
      vim.list_extend(args, { '-I', path })
    end
    vim.list_extend(args, { '-f', opts.index })
    return args
  end
  local args = { opts.exec, '-bk', '-f', opts.index }
  if opts.recursive then args[#args + 1] = '-R' end
  for _, path in ipairs(opts.include_dirs) do
    vim.list_extend(args, { '-I', path })
  end
  if opts.file_list then vim.list_extend(args, { '-i', opts.file_list }) end
  if #opts.sources > 0 then
    args[#args + 1] = '--'
    vim.list_extend(args, opts.sources)
  end
  return args
end

function M.status()
  return vim.deepcopy(last)
end

function M.find(field, pattern)
  assert(not running, 'Ripscope operation is already running')
  local entry = current()
  field = fields[field] or tonumber(field)
  assert(valid[field], 'unsupported query field')
  assert(type(pattern) == 'string' and not pattern:find('[%z\r\n]') and #pattern <= 4096,
    'pattern must be one line of at most 4096 UTF-8 bytes')
  local function query()
    start(entry, 'query', field .. pattern, 'Ripscope ' .. field .. ' ' .. pattern)
  end
  local opts = entry.config
  if entry.automatic and not entry.published then
    -- The first search in a project indexes it, as does the next after a first index
    -- build that never published.
    vim.notify('Ripscope: indexing ' .. opts.root .. ' for the first time', vim.log.levels.INFO)
    start(entry, 'build', build_args(opts), 'Ripscope build ' .. opts.index, function()
      entry.published = true
      query()
    end)
  else
    query()
  end
end

function M.build()
  local entry = current()
  start(entry, 'build', build_args(entry.config), 'Ripscope build ' .. entry.config.index, function()
    entry.published = true
  end)
end

-- Installs the executable of this checkout's version with the repository's installer,
-- when none is found or the one found is older, or always with FORCE.
function M.install(force)
  local source = debug.getinfo(1, 'S').source:sub(2)
  local repository = vim.fs.dirname(vim.fs.dirname(vim.fs.normalize(source)))
  local wanted = project.checkout_version(repository)
  local found = project.executable()
  if found and not force then
    local result = vim.system({ found, '--version' }, { text = true }):wait()
    local have = (result.stdout or ''):match('ripscope (%S+)')
    -- Only an executable the installer put there is upgraded, not a system package's.
    local ours = project.same(found, project.installer_executable())
    if result.code == 0 and have and not (wanted and ours and project.older(have, wanted)) then
      vim.notify('Ripscope: ripscope ' .. have .. ' is installed: ' .. found, vim.log.levels.INFO)
      return
    end
  end
  local script = repository .. (windows and '/install.ps1' or '/install.sh')
  assert(vim.uv.fs_stat(script), 'this copy has no installer; run the one-line installer from https://ripscope.com')
  local command = windows and { (vim.env.SystemRoot or 'C:/Windows') .. '/System32/WindowsPowerShell/v1.0/powershell.exe',
    '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script } or { 'sh', script }
  vim.notify('Ripscope: installing ripscope ' .. (wanted or '') .. '...', vim.log.levels.INFO)
  -- The plugin is here already, so the installer leaves the editors alone.
  local env = { RIPSCOPE_EDITORS = 'none', RIPSCOPE_VERSION = wanted }
  local result = vim.system(command, { cwd = repository, text = true, env = env }):wait()
  local output = (result.stdout or '') .. (result.stderr or '')
  log('Ripscope install\n' .. output)
  if result.code ~= 0 then
    error('installation failed (exit ' .. tostring(result.code) .. '); see :RipscopeLog\n' .. output)
  end
  vim.notify('Ripscope: ' .. (output:match('(Installed[^\n]*)') or 'installed'), vim.log.levels.INFO)
end

function M.commands()
  vim.api.nvim_create_user_command('RipscopeBuild', function() M.build() end, {})
  vim.api.nvim_create_user_command('RipscopeFind', function(command)
    local field, pattern = command.args:match('^(%S+)%s+(.*)$')
    assert(field, 'usage: RipscopeFind FIELD PATTERN')
    M.find(field, pattern)
  end, { nargs = '+' })
  vim.api.nvim_create_user_command('RipscopeLog', M.show_log, {})
  vim.api.nvim_create_user_command('RipscopeInstall', function(command) M.install(command.bang) end, { bang = true })
end

function M.setup(options)
  assert(not running, 'cannot reconfigure while a Ripscope operation is running')
  local opts = settings(options)
  if opts.root == nil then
    -- Automatic projects, with these settings.
    if opts.exec then opts.exec = executable(opts.exec) end
    close_sessions()
    explicit, defaults, projects, located, recent = nil, vim.deepcopy(options or {}), {}, {}, nil
    M.commands()
    return
  end
  opts.root = absolute(string_arg(opts.root, 'root'), vim.fn.getcwd())
  assert(vim.fn.isdirectory(opts.root) == 1, 'root must be an existing directory')
  opts.index = absolute(opts.index or 'ripscope.db', opts.root)
  opts.exec = executable(opts.exec)
  close_sessions()
  explicit = { config = opts }
  defaults, projects, located, recent = {}, { [opts.root .. '\0' .. opts.index] = explicit }, {}, nil
  M.commands()
end

vim.api.nvim_create_autocmd('VimLeavePre', {
  group = vim.api.nvim_create_augroup('RipscopeSession', { clear = true }),
  callback = close_sessions,
})

return M
