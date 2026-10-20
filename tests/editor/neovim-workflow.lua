vim.opt.runtimepath:append(vim.fs.dirname(vim.fs.dirname(vim.env.RIPSCOPE_NVIM_ADAPTER)))
local adapter = require('ripscope')
local backend = vim.env.RIPSCOPE_NVIM_BACKEND
local windows = vim.fn.has('win32') == 1
local function normalized(path)
  return vim.fs.normalize(path, { expand_env = false })
end
local test_root = normalized(vim.env.RIPSCOPE_NVIM_TEST_ROOT)
local temporary = {}
local passed = 0
local session_pids = {}
local native_system = vim.system
local native_jobs = {}

vim.system = function(command, options, on_exit)
  local job = { complete = false }
  local process = native_system(command, options, function(result)
    job.complete = true
    if on_exit then on_exit(result) end
  end)
  job.pid = process.pid
  if vim.tbl_contains(command, '--json') and vim.tbl_contains(command, '-dl') then
    print('SESSION_ARGV ' .. vim.json.encode(command))
  end
  native_jobs[#native_jobs + 1] = job
  return process
end

local function check(condition, message)
  assert(condition, message or 'assertion failed')
end
local function remove_fixture(path)
  local function removed(operation, ok, message, code)
    if not ok then
      local failure = { operation = operation, path = path, error = message, code = code }
      io.stderr:write('CLEANUP_FAILURE ' .. vim.json.encode(failure) .. '\n')
      error('fixture cleanup failed: ' .. vim.inspect(failure))
    end
    return ok
  end
  local stat = removed('lstat', vim.uv.fs_lstat(path))
  if stat.type == 'directory' then
    local entries = removed('scandir', vim.uv.fs_scandir(path))
    while true do
      local name = vim.uv.fs_scandir_next(entries)
      if not name then break end
      remove_fixture(path .. '/' .. name)
    end
    removed('rmdir', vim.uv.fs_rmdir(path))
  else
    removed('unlink', vim.uv.fs_unlink(path))
  end
end
local function write(path, text)
  local file = assert(vim.uv.fs_open(path, 'w', 384))
  assert(vim.uv.fs_write(file, text, 0))
  assert(vim.uv.fs_close(file))
end
local function project()
  local root = test_root .. '/project-' .. (#temporary + 1) .. ' ü & space'
  vim.fn.mkdir(root, 'p')
  temporary[#temporary + 1] = root
  write(root .. '/case.h', 'extern int value;\n')
  write(root .. '/case ü.c', '#include "case.h"\nint value;\nint callee(void) { return value; }\nint caller(void) { value = 7; return callee(); }\n')
  return root
end
local function setup(root, more)
  adapter.setup(vim.tbl_extend('force', { exec = backend, root = root, index = 'index ü', open = false }, more or {}))
end
local function wait(ok)
  check(vim.wait(15000, function() return adapter.status().state == 'complete' end, 10), 'operation hung')
  local result = adapter.status()
  if result.session_pid then
    session_pids[result.session_pid] = true
    print('SESSION_PID ' .. result.session_pid)
  end
  print(result.message)
  if ok ~= nil then check(result.ok == ok, vim.inspect(result)) end
  return result
end
local function query(field, pattern, ok)
  adapter.find(field, pattern)
  local result = wait(ok ~= false)
  return vim.fn.getqflist({ id = result.quickfix_id, items = 1 }).items, result
end
local function jump(item, root, file, line)
  check(normalized(vim.api.nvim_buf_get_name(item.bufnr)) == root .. '/' .. file, vim.inspect(item))
  check(item.lnum == line, vim.inspect(item))
  vim.cmd('cfirst')
  check(normalized(vim.api.nvim_buf_get_name(0)) == root .. '/' .. file, 'wrong jump file')
  check(vim.api.nvim_win_get_cursor(0)[1] == line, 'wrong jump line')
end
local function test(name, body)
  body()
  passed = passed + 1
  print('PASS ' .. name)
end
local function fake(root, body)
  local path = root .. '/fake-backend'
  write(path, '#!/bin/sh\n' .. body .. '\n')
  check(vim.uv.fs_chmod(path, 448))
  return path
end

local ok, error = xpcall(function()
  test('saved and external changes refresh before navigation in one session', function()
    local root = project()
    setup(root)
    adapter.build(); wait(true)
    local _, first = query(1, 'callee')
    local function current(field, pattern, file, line, shown)
      local items, result = query(field, pattern)
      check(result.session_pid == first.session_pid, 'automatic Refresh restarted the session')
      check(#items == 1, vim.inspect(items))
      check(items[1].text:find(shown, 1, true), 'displayed result did not update')
      vim.cmd('checktime')
      jump(items[1], root, file, line)
      check(vim.api.nvim_get_current_line():find(shown, 1, true), 'navigation opened stale source')
    end
    vim.o.autoread = true
    vim.cmd('edit ' .. vim.fn.fnameescape(root .. '/case ü.c'))
    vim.api.nvim_buf_set_lines(0, 0, -1, false,
      { '#include "case.h"', 'int Saved;', 'void changed(void) { target(); }' })
    vim.cmd('write')
    current(1, 'Saved', 'case ü.c', 2, 'Saved')
    current(3, 'target', 'case ü.c', 3, 'changed')
    write(root .. '/case ü.c', '#include "case.h"\nint Saved;\nvoid changed(void) { target(); }\nint Outside;\n')
    current(1, 'Outside', 'case ü.c', 4, 'Outside')
    write(root .. '/case.h', 'int HeaderFresh;\n')
    current(1, 'HeaderFresh', 'case.h', 1, 'HeaderFresh')
    write(root .. '/added.c', 'int Added;\n')
    current(1, 'Added', 'added.c', 1, 'Added')
    check(vim.uv.fs_rename(root .. '/added.c', root .. '/renamed.c'))
    current(1, 'Added', 'renamed.c', 1, 'Added')
    check(vim.fn.delete(root .. '/renamed.c') == 0)
    local items, removed = query(1, 'Added')
    check(#items == 0 and removed.session_pid == first.session_pid, 'removed file stayed indexed')
  end)

  test('all nine fields, commands, quickfix jumps and captured cwd', function()
    local root = project()
    setup(root)
    vim.api.nvim_set_current_dir(vim.env.RIPSCOPE_NVIM_OTHER_CWD)
    vim.cmd('RipscopeBuild')
    wait(true)
    local cases = { { 's', 'value', 2 }, { 'g', 'callee', 3 }, { 'd', 'caller', 4 },
      { 'c', 'callee', 4 }, { 't', 'value = 7', 4 }, { 'e', 'value.*7', 4 },
      { 'f', 'case ü.c', 1 }, { 'i', 'case.h', 1 }, { 'a', 'value', 4 } }
    for _, case in ipairs(cases) do
      vim.cmd('RipscopeFind ' .. case[1] .. ' ' .. case[2])
      wait(true)
      local items = vim.fn.getqflist()
      check(#items > 0)
      jump(items[1], root, 'case ü.c', case[3])
    end
    local items, result = query(1, 'Absent')
    check(#items == 0 and result.message == 'No matches')
  end)

  test('automatic Refresh respects fixed lists and observes list changes', function()
    local root = project()
    local selection = '"case ü.c"\n'
    write(root .. '/selection list', selection)
    setup(root, { file_list = 'selection list' })
    adapter.build(); wait(true)
    local _, first = query(1, 'callee')
    check(vim.fn.delete(root .. '/selection list') == 0)
    local retained, uncertain = query(1, 'callee')
    check(#retained == 1 and uncertain.session_pid == first.session_pid)
    check(uncertain.message:find('freshness is unconfirmed', 1, true), 'automatic failure was hidden')
    check(uncertain.stderr:find('automatic Refresh did not complete', 1, true), 'missing failure detail')
    jump(retained[1], root, 'case ü.c', 3)
    write(root .. '/selection list', selection)
    local _, repaired = query(1, 'callee')
    check(repaired.stderr == '', 'restored list retained the previous warning')
    write(root .. '/listed.c', 'int Listed;\n')
    check(#query(1, 'Listed') == 0, 'unlisted file was discovered')
    check(table.concat(vim.fn.readfile(root .. '/selection list'), '\n') .. '\n' == selection,
      'query overwrote the Selection rule')
    write(root .. '/selection list', selection .. 'listed.c\n')
    local items, changed = query(1, 'Listed')
    check(changed.session_pid == first.session_pid, 'list update restarted the session')
    jump(items[1], root, 'listed.c', 1)
    write(root .. '/selection list', selection)
    check(#query(1, 'Listed') == 0, 'removed list entry stayed indexed')
    setup(root, { sources = { 'case ü.c' } })
    adapter.build(); wait(true)
    check(#query(1, 'Listed') == 0, 'explicit source list expanded on save')
  end)

  test('stored queries, a build past a file it cannot use, retry and new-content jump', function()
    local root = project()
    setup(root, { auto_refresh = false })
    adapter.build(); wait(true)
    write(root .. '/added.c', 'int Added;\n')
    check(#query(1, 'Added') == 0, 'query implicitly refreshed')
    local original = table.concat(vim.fn.readfile(root .. '/case ü.c'), '\n') .. '\n'
    if windows then
      setup(root, { sources = { 'case ü.c', 'added.c' }, auto_refresh = false })
      check(vim.fn.delete(root .. '/case ü.c') == 0)
    else
      check(vim.uv.fs_chmod(root .. '/case ü.c', 0))
    end
    -- As cscope, the build carries on: it names the file it could not use, which leaves
    -- the index until it can be read again.
    adapter.build()
    local built = wait(true)
    check(built.stderr:find('case ü.c', 1, true), vim.inspect(built))
    check(built.message:find('case ü.c', 1, true), 'warning not shown')
    check(#query(1, 'callee') == 0, 'a file the build could not use was kept')
    if windows then write(root .. '/case ü.c', original)
    else check(vim.uv.fs_chmod(root .. '/case ü.c', 384)) end
    adapter.build(); wait(true)
    check(#query(1, 'callee') > 0, 'the file was not indexed again')
    local items = query(1, 'Added')
    jump(items[1], root, 'added.c', 1)
  end)

  test('one session survives queries, errors, warnings and index replacement', function()
    local root = project()
    setup(root, { auto_refresh = false })
    adapter.build(); wait(true)
    local _, first = query(1, 'callee')
    check(type(first.session_pid) == 'number', 'query did not use a persistent session')
    local function same(result)
      check(result.session_pid == first.session_pid, 'query restarted the session')
    end
    local _, empty = query(1, 'Absent'); same(empty)
    local _, refused = query(1, '[', false); same(refused)
    check(refused.stderr:find('invalid query pattern', 1, true), 'missing query diagnostic')
    check(#query(1, 'callee') == 1, 'session did not recover after a refused query')
    check(vim.fn.delete(root .. '/case ü.c') == 0)
    local available, warning = query(4, 'value'); same(warning)
    check(#available == 1, 'warning discarded the readable file result')
    jump(available[1], root, 'case.h', 1)
    check(warning.stderr:find('case ü.c', 1, true), 'warning was lost')
    check(warning.message:find('case ü.c', 1, true), 'warning was not shown')
    local absent, incomplete = query(4, 'Absent'); same(incomplete)
    check(#absent == 0, 'unexpected result for an absent name')
    check(incomplete.message:find('case ü.c', 1, true), 'empty answer hid its warning')
    write(root .. '/case ü.c', 'int Replacement;\n')
    local rebuilt = vim.system({ backend, '-bku', '-f', root .. '/index ü' }, { cwd = root }):wait()
    check(rebuilt.code == 0, rebuilt.stderr)
    local items, replacement = query(1, 'Replacement'); same(replacement)
    check(#items == 1, 'session kept the previous index')
    local _, repaired = query(4, 'Replacement'); same(repaired)
    check(repaired.stderr == '', 'repaired source kept a previous warning')
  end)

  test('completed deadlines cannot cancel a later query and dead sessions restart', function()
    local root = project()
    setup(root)
    adapter.build(); wait(true)
    -- Control the clock boundary: invoke a callback that was queued before cancellation.
    local new_timer, callbacks = vim.uv.new_timer, {}
    vim.uv.new_timer = function()
      local timer = new_timer()
      return setmetatable({
        start = function(_, timeout, repeating, callback)
          if timeout == 120000 then callbacks[#callbacks + 1] = callback end
          timer:start(timeout, repeating, callback)
        end,
        stop = function() timer:stop() end,
        close = function() timer:close() end,
      }, { __index = function(_, key)
        return function(_, ...) return timer[key](timer, ...) end
      end })
    end
    local ok, problem = pcall(function()
      local _, first = query(1, 'callee')
      adapter.find(1, 'caller')
      callbacks[1]()
      local second = wait(true)
      check(second.session_pid == first.session_pid, 'stale deadline killed the session')
    end)
    vim.uv.new_timer = new_timer
    check(ok, tostring(problem))
    local pid = adapter.status().session_pid
    check(vim.uv.kill(pid, 9))
    check(vim.wait(3000, function() return not vim.uv.kill(pid, 0) end, 10), 'backend did not exit')
    adapter.find(1, 'callee')
    local restarted = wait()
    if not restarted.ok then
      check(#vim.fn.getqflist({ id = restarted.quickfix_id, items = 1 }).items == 0)
      _, restarted = query(1, 'callee') -- No automatic replay of a query racing process death.
    end
    local items = vim.fn.getqflist({ id = restarted.quickfix_id, items = 1 }).items
    check(#items == 1 and restarted.session_pid ~= pid, 'dead session was not replaced')
    setup(root)
    check(vim.wait(3000, function() return not vim.uv.kill(restarted.session_pid, 0) end, 10),
      'reconfiguration left its old process alive')
  end)

  test('missing and invalid indexes differ from no matches and show diagnostics', function()
    local root = project()
    setup(root, { open = true, auto_refresh = false })
    local items, result = query(1, 'callee', false)
    check(#items == 0 and #result.stderr > 0 and result.message ~= 'No matches')
    check(vim.api.nvim_buf_get_name(0) == 'ripscope://log', 'failure log not visible')
    check(table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n'):find(result.stderr, 1, true))
    write(root .. '/index ü', 'not an index\n')
    items, result = query(1, 'callee', false)
    check(#items == 0 and #result.stderr > 0)
    vim.cmd('close')
  end)

  test('selection, recursion and include paths', function()
    local root = project()
    vim.fn.mkdir(root .. '/nested', 'p')
    write(root .. '/nested/deep.c', 'int Deep;\n')
    setup(root)
    adapter.build(); wait(true)
    check(#query(1, 'Deep') == 0)
    setup(root, { recursive = true })
    adapter.build(); wait(true)
    check(#query(1, 'Deep') == 1)
    setup(root, { sources = { 'nested/deep.c' } })
    adapter.build(); wait(true)
    check(#query(1, 'Deep') == 1 and #query(1, 'callee') == 0)
    write(root .. '/selection list', '"case ü.c"\n')
    setup(root, { file_list = 'selection list' })
    adapter.build(); wait(true)
    check(#query(1, 'callee') == 1 and #query(1, 'Deep') == 0)
    vim.fn.mkdir(root .. '/include dir', 'p')
    write(root .. '/include dir/extra.h', 'int HeaderValue;\n')
    write(root .. '/included.c', '#include "extra.h"\nint Local;\n')
    setup(root, { sources = { 'included.c' }, include_dirs = { 'include dir' } })
    adapter.build(); wait(true)
    check(#query(1, 'HeaderValue') == 1)
  end)

  if not windows then
  test('Linux strict framing, typed rows, byte text and no partial results', function()
    local root = project()
    local good = '["case ü.c","callee",3,"int callee"]\n'
    local header, ready = '{"type":"result","count":1}\n', '{"type":"ready"}\n'
    local invalid = { good .. '[broken\n', good:sub(1, -2), '["", "ctx", 1, "text"]\n',
      '["case.c", "ctx", 0, "text"]\n', '["case.c", "ctx", 1.5, "text"]\n',
      '["case.c", "ctx", 1, [256]]\n', '["case.c", "ctx", 1, {}]\n',
      '["case.c", "ctx", 1, "text", "extra"]\n' }
    local path = root .. '/response'
    local exec = fake(root, 'printf \'{"ty\'; sleep 0.01; printf \'pe":"ready","extra":true}\\n\'\n'
      .. 'while IFS= read -r request; do cat "' .. path .. '"; done')
    setup(root, { exec = exec, timeout = 500 })
    for _, text in ipairs(invalid) do
      write(path, header .. text .. ready)
      local items, result = query(1, 'callee', false)
      check(#items == 0 and result.message:find('Invalid query response', 1, true))
    end
    for _, text in ipairs({ header .. good, header .. ready, header .. good .. good .. ready,
      '{"type":"result","count":-1}\n' .. ready,
      '{"type":"result","count":1.5}\n' .. ready,
      '{"type":"warning","message":"too early"}\n' .. ready }) do
      write(path, text)
      local items = query(1, 'callee', false)
      check(#items == 0, 'partial reply was published')
    end
    write(path, header .. '["case ü.c","ctx",1,[255,10,65]]\n' .. ready)
    local items = query(1, 'x')
    check(items[1].text == '<<ctx>> \\xff\\x0a\\x41')
    write(path, '{"type":"error","message":"refused"}\n'
      .. '{"type":"warning","message":"detail"}\n' .. ready)
    local _, refused = query(1, 'x', false)
    check(refused.stderr:find('refused', 1, true) and refused.stderr:find('detail', 1, true))
  end)

  test('async exclusion and quickfix list ownership', function()
    local root = project()
    local exec = fake(root, [[printf '{"type":"ready"}\n'
IFS= read -r request
sleep 0.2
printf '{"type":"result","count":1}\n["case ü.c","ctx",1,"ok"]\n{"type":"ready"}\n']])
    setup(root, { exec = exec, open = true })
    adapter.find(1, 'x')
    local id = adapter.status().quickfix_id
    check(not pcall(adapter.build), 'concurrent build allowed')
    check(not pcall(setup, root), 'busy reconfiguration allowed')
    vim.fn.setqflist({}, ' ', { title = 'unrelated', items = { { filename = '/tmp/other', lnum = 1, text = 'keep' } } })
    local other = vim.fn.getqflist({ id = 0 }).id
    wait(true)
    check(vim.fn.getqflist({ id = 0 }).id == other)
    check(vim.fn.getqflist()[1].text == 'keep')
    check(#vim.fn.getqflist({ id = id, items = 1 }).items == 1)
  end)

  test('launch failure, exit status, signal, timeout and reuse', function()
    local root = project()
    local exec = fake(root, 'printf tail-diagnostic >&2\nexit 7')
    setup(root, { exec = exec })
    local _, result = query(1, 'x', false)
    check(result.code == 7 and result.stderr == 'tail-diagnostic')
    fake(root, 'exec 1>&-\nsleep 0.02\nprintf late-diagnostic >&2\nexit 7')
    _, result = query(1, 'x', false)
    check(result.code == 7 and result.stderr == 'late-diagnostic', 'stderr after EOF was lost')
    fake(root, 'kill -TERM $$')
    _, result = query(1, 'x', false)
    check(result.signal ~= 0)
    fake(root, 'exec sleep 10')
    setup(root, { exec = exec, timeout = 50 })
    _, result = query(1, 'x', false)
    check(result.code == 124, 'query deadline did not stop the session')
    adapter.build(); result = wait(false)
    check(result.message:find('unconfirmed', 1, true))
    setup(root, { exec = exec })
    vim.fn.delete(exec)
    query(1, 'x', false)
    setup(root)
    adapter.build(); wait(true)
    check(#query(1, 'callee') == 1)
    check(not pcall(setup, root, { exec = '/no/such/backend' }))
    check(not pcall(setup, root, { sources = { 'x' }, file_list = 'y' }))
    check(not pcall(adapter.find, 5, 'x'))
    check(not pcall(adapter.find, 1, 'x\ny'))
  end)
  else
  test('Windows denied output, visible failure and successful retry', function()
    local root = project()
    setup(root, { index = vim.env.RIPSCOPE_NVIM_DENIED .. '/index', open = true })
    adapter.build()
    local result = wait(false)
    check(result.code ~= 0 and #result.stderr > 0)
    check(vim.api.nvim_buf_get_name(0) == 'ripscope://log')
    vim.cmd('close')
    write(root .. '/new.c', 'int AfterDenied;\n')
    local restored = vim.system({ vim.env.RIPSCOPE_NVIM_ICACLS, vim.env.RIPSCOPE_NVIM_DENIED,
      '/remove:d', '*' .. vim.env.RIPSCOPE_NVIM_SID }):wait()
    check(restored.code == 0, restored.stderr)
    adapter.build(); wait(true)
    jump(query(1, 'AfterDenied')[1], root, 'new.c', 1)
  end)

  test('Windows drive forms, absolute result paths, quoting and ambiguous-path refusal', function()
    local root = project()
    local function backslashes(path) return (path:gsub('/', '\\')) end
    setup(backslashes(root), { exec = backslashes(backend), index = backslashes(root .. '/drive index'),
      sources = { backslashes(root .. '/case ü.c') } })
    adapter.build(); wait(true)
    jump(query(1, 'callee')[1], root, 'case ü.c', 3)
    setup(root, { index = root .. '/drive index', sources = { root .. '/case ü.c' } })
    adapter.build(); wait(true)
    jump(query(1, 'callee')[1], root, 'case ü.c', 3)
    local _, result = query(4, 'no "match" & echo %PATH%')
    check(result.message == 'No matches', 'pattern was interpreted by a shell')
    -- UNC syntax is checked without contacting or qualifying network storage.
    local system, captured = vim.system, nil
    setup(root, { index = '\\\\server\\share\\index' })
    vim.system = function(args, _, callback)
      captured = args
      callback({ code = 0, signal = 0, stdout = '', stderr = '' })
    end
    local started, problem = pcall(adapter.build)
    vim.system = system
    check(started, tostring(problem))
    wait(true)
    check(captured[4] == '//server/share/index', 'UNC prefix was changed')
    for _, path in ipairs({ 'C:relative', '\\rooted', '/rooted', '\\\\?\\C:\\device',
      '//./C:/device', '//server', '\\\\server\\' }) do
      check(not pcall(setup, root, { index = path }), 'ambiguous path accepted: ' .. path)
    end
    local script = root .. '/backend.CMD'
    write(script, '@exit /b 0\r\n')
    check(not pcall(setup, root, { exec = script }))
  end)

  test('Windows async ownership, launch failure, native timeout and retry', function()
    local root = project()
    setup(root)
    adapter.build(); wait(true)
    adapter.find(1, 'callee')
    local id = adapter.status().quickfix_id
    check(not pcall(adapter.build))
    check(not pcall(setup, root))
    vim.fn.setqflist({}, ' ', { title = 'other producer', items = { { filename = root .. '/other', lnum = 1, text = 'keep' } } })
    local other = vim.fn.getqflist({ id = 0 }).id
    wait(true)
    check(vim.fn.getqflist({ id = 0 }).id == other and vim.fn.getqflist()[1].text == 'keep')
    check(#vim.fn.getqflist({ id = id, items = 1 }).items == 1)
    local copy = root .. '/vanishing.exe'
    check(vim.uv.fs_copyfile(backend, copy))
    setup(root, { exec = copy })
    check(vim.fn.delete(copy) == 0)
    query(1, 'callee', false)
    check(not pcall(setup, root, { exec = root .. '/absent.exe' }))
    setup(root, { timeout = 1 })
    adapter.build(); local result = wait(false)
    check(result.message:find('unconfirmed', 1, true))
    setup(root)
    adapter.build(); wait(true)
    check(#query(1, 'callee') == 1)
  end)
  end

  test('literal environment-variable names in root, index and source paths', function()
    local base = project()
    local root = base .. '/$HOME project'
    vim.fn.mkdir(root, 'p')
    write(root .. '/$HOME.c', 'int LiteralDollar;\n')
    setup(root, { index = '$HOME index' })
    adapter.build(); wait(true)
    check(vim.fn.filereadable(root .. '/$HOME index') == 1)
    local items = query(1, 'LiteralDollar')
    jump(items[1], root, '$HOME.c', 1)
  end)

  test('publication error releases operation ownership', function()
    local root = project()
    setup(root)
    adapter.build(); wait(true)
    adapter.find(1, 'callee')
    local original = vim.fn.setqflist
    vim.fn.setqflist = function() error('injected quickfix publication failure') end
    local completed, result = pcall(wait, false)
    vim.fn.setqflist = original
    check(completed, tostring(result))
    check(result.message:find('publication failure', 1, true))
    check(#query(1, 'callee') == 1)
  end)
end, debug.traceback)

vim.api.nvim_set_current_dir(test_root)
vim.api.nvim_exec_autocmds('VimLeavePre', {})
vim.system = native_system
check(vim.wait(3000, function()
  -- Native exit callbacks can follow the OS PID disappearing.
  for _, job in ipairs(native_jobs) do if not job.complete then return false end end
  for pid in pairs(session_pids) do if vim.uv.kill(pid, 0) then return false end end
  return true
end, 10), 'native job cleanup did not complete: ' .. vim.inspect(native_jobs))
print('SHUTDOWN_COMPLETE ' .. vim.json.encode({ native_jobs = #native_jobs }))
local cleaned, cleanup_error = xpcall(function()
  for _, path in ipairs(temporary) do remove_fixture(path) end
end, debug.traceback)
if not ok then
  io.stderr:write(tostring(error) .. '\n')
end
if not cleaned then
  io.stderr:write(tostring(cleanup_error) .. '\n')
end
if not ok or not cleaned then
  vim.cmd('cquit 1')
end
print(string.format('PASS %d Neovim workflows', passed))
vim.cmd('qa!')
