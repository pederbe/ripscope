-- One JSON session per configured project. No result is complete before its ready frame.
local M = {}

function M.new(opts, decode_row)
  local connection
  local client = {}

  local function stop_timer(op)
    if op.timer then
      op.timer:stop()
      op.timer:close()
      op.timer = nil
    end
  end

  function client.close()
    local old = connection
    connection = nil
    if old then
      if old.active then stop_timer(old.active) end
      if old.process then
        pcall(old.process.kill, old.process, 9)
      end
    end
  end

  local function finish(c, code, signal, message)
    local op = c.active
    if not op then return end
    stop_timer(op)
    c.active = nil
    op.complete({ code = code, signal = signal, message = message,
      stdout = table.concat(op.rows), stderr = table.concat(c.stderr) .. table.concat(op.notes),
      items = op.items, session_pid = c.process and c.process.pid })
    c.stderr = {}
  end

  local function fail(c, message, code, signal)
    if connection ~= c then return end
    client.close()
    finish(c, code or -1, signal or 0, message)
  end

  local function send(c)
    c.phase = 'result'
    local ok, err = pcall(c.process.write, c.process, c.active.request .. '\n')
    if not ok then fail(c, 'Query write failed: ' .. tostring(err)) end
  end

  local function frame(c, line)
    local value = vim.json.decode(line)
    assert(type(value) == 'table', 'expected a JSON frame')
    local op = c.active
    if value.type == 'ready' then
      if c.phase == 'starting' then
        send(c)
      else
        assert(c.phase == 'ready', 'ready before the response was complete')
        c.phase = 'idle'
        finish(c, op.error and 1 or 0, 0, op.error and ('Query failed: ' .. op.error))
      end
    elseif value.type == 'result' then
      assert(c.phase == 'result', 'unexpected result header')
      local count = value.count
      assert(type(count) == 'number' and count >= 0 and count < math.huge
        and count == math.floor(count), 'invalid result count')
      op.remaining = count
      c.phase = count == 0 and 'ready' or 'rows'
    elseif value.type == 'error' then
      assert(c.phase == 'result' and type(value.message) == 'string', 'unexpected error frame')
      op.error = value.message
      op.notes[#op.notes + 1] = 'ripscope: ' .. value.message .. '\n'
      c.phase = 'ready'
    elseif value.type == 'warning' then
      assert(c.phase == 'ready' and type(value.message) == 'string',
        'unexpected warning frame')
      op.notes[#op.notes + 1] = 'ripscope: ' .. value.message .. '\n'
    else
      assert(c.phase == 'rows', 'unexpected row or frame')
      op.items[#op.items + 1] = decode_row(value)
      op.rows[#op.rows + 1] = line .. '\n'
      op.remaining = op.remaining - 1
      if op.remaining == 0 then c.phase = 'ready' end
    end
  end

  local function output(c, err, data)
    if connection ~= c then return end
    if err then fail(c, 'Query output failed: ' .. tostring(err)); return end
    if not data then
      c.eof = true -- Drain stderr and keep the real exit status in the exit callback.
      return
    end
    local from = 1
    while connection == c do
      local ending = data:find('\n', from, true)
      if not ending then
        c.fragments[#c.fragments + 1] = data:sub(from)
        break
      end
      c.fragments[#c.fragments + 1] = data:sub(from, ending - 1)
      local line = table.concat(c.fragments)
      c.fragments = {}
      local ok, problem = pcall(frame, c, line)
      if not ok then fail(c, 'Invalid query response: ' .. tostring(problem)); return end
      from = ending + 1
      if from > #data then break end
    end
  end

  function client.query(request, complete)
    local c = connection
    if c and (c.eof or c.process:is_closing()) then client.close(); c = nil end
    if not c then
      c = { phase = 'starting', fragments = {}, stderr = {} }
      connection = c
    end
    assert(not c.active, 'session query already running')
    local op = { request = request, complete = complete, rows = {}, items = {}, notes = {} }
    c.active = op
    op.timer = assert(vim.uv.new_timer())
    op.timer:start(opts.timeout, 0, vim.schedule_wrap(function()
      if connection == c and c.active == op then fail(c, 'Query timed out', 124, 0) end
    end))
    if c.process then
      send(c)
      return
    end
    local command = { opts.exec, '--json', '-dl', opts.auto_refresh and '--auto-refresh' or '--no-auto-refresh' }
    vim.list_extend(command, opts.session_args or {})
    vim.list_extend(command, { '-f', opts.index, '-P', opts.root })
    local launched, process = pcall(vim.system, command,
      { cwd = opts.root, text = true, stdin = true,
        stdout = function(err, data) vim.schedule(function() output(c, err, data) end) end,
        stderr = function(err, data)
          vim.schedule(function()
            if connection ~= c then return end
            if err then fail(c, 'Query diagnostics failed: ' .. tostring(err))
            elseif data then c.stderr[#c.stderr + 1] = data end
          end)
        end },
      function(result)
        vim.schedule(function()
          if connection ~= c then return end
          local message = string.format('Query session ended (exit %s, signal %s)', result.code, result.signal)
          fail(c, message, result.code == 0 and -1 or result.code, result.signal)
        end)
      end)
    if launched then c.process = process
    else
      c.stderr[#c.stderr + 1] = tostring(process)
      fail(c, 'Query launch failed: ' .. tostring(process))
    end
  end

  return client
end

return M
