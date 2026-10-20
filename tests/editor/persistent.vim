" Run only through the check-vim example in its disposable working directory.
set nomore
if has('win32')
  set shellslash
endif
set cscopeverbose
set cscopequickfix=s-,g-,d-,c-,f-,t-,e-,i-,a-
let &cscopeprg = $RIPSCOPE_BACKEND . ' --vim-errors'
let s:root = getcwd()
let s:observations = []

function! s:Connect() abort
  execute 'cs add ' . fnameescape(s:root . '/index') . ' ' . fnameescape(s:root)
  call assert_equal(1, cscope_connection(), 'startup readiness')
  let connection = execute('cs show')
  let s:pid = matchstr(connection, '\n\s*0\s\+\zs\d\+')
  call assert_notequal('', s:pid, 'backend PID')
  call add(s:observations, {'connection': connection})
endfunction

function! s:Disconnect() abort
  cs kill -1
  call assert_equal(0, cscope_connection(), 'disconnect')
  if !has('win32')
    call assert_false(isdirectory('/proc/' . s:pid), 'backend exited on disconnect')
  endif
endfunction

function! s:Query(field, pattern, expected) abort
  call setqflist([], 'r')
  let v:errmsg = ''
  let output = execute('silent! cs find ' . a:field . ' ' . a:pattern)
  let rows = []
  let navigation = []
  for entry in getqflist()
    call add(rows, [fnamemodify(bufname(entry.bufnr), ':t'), entry.lnum])
    call assert_equal(1, entry.valid)
    call assert_equal(s:root . '/' . rows[-1][0], fnamemodify(bufname(entry.bufnr), ':p'))
  endfor
  call assert_equal(a:expected, rows, a:field . ' ' . a:pattern)
  for i in range(len(rows))
    execute 'cc ' . (i + 1)
    let position = [expand('%:t'), line('.')]
    call add(navigation, position)
    call assert_equal(rows[i], position, 'navigation')
  endfor
  call add(s:observations, {'request': a:field . ' ' . a:pattern,
        \ 'rows': rows, 'navigation': navigation, 'messages': output,
        \ 'error': v:errmsg})
  call assert_equal(1, cscope_connection(), 'connection survives query')
  call assert_equal(s:pid, matchstr(execute('cs show'), '\n\s*0\s\+\zs\d\+'), 'same backend')
endfunction

try
  call assert_true(has('cscope') && has('quickfix'))
  execute 'cd ' . fnameescape(s:root . '/elsewhere')
  call s:Connect()
  call s:Query('g', 'Alpha', [['alpha.c', 1]])
  call s:Query('s', 'Alpha', [['alpha.c', 1], ['beta.c', 2]])
  call s:Query('g', 'Alpha|Beta', [['alpha.c', 1], ['beta.c', 1]])
  call s:Query('f', 'alpha', [['alpha.c', 1]])
  call s:Query('f', '\.c$', [['alpha.c', 1], ['beta.c', 1]])
  call s:Query('f', '^beta\.c$', [['beta.c', 1]])
  call s:Query('i', 'shared', [['alpha.c', 2], ['beta.c', 3]])
  call s:Query('i', '^unique\.h$', [['beta.c', 4]])
  call s:Query('c', 'Callee', [['alpha.c', 3], ['beta.c', 5]])
  call s:Query('c', '^Callee$', [['alpha.c', 3], ['beta.c', 5]])
  call s:Query('c', 'callback', [])
  call s:Query('d', 'call_alpha', [['alpha.c', 3]])
  " The written function owns its body; callback is a parameter.
  call s:Query('d', 'call_.*', [['alpha.c', 3], ['beta.c', 5]])
  call s:Query('d', 'callback', [])
  for field in ['s', 'g', 'd', 'c', 'f', 'e', 'i', 'a']
    call s:Query(field, 'Missing', [])
    call assert_match('E259:', s:observations[-1].error, 'visible empty response')
    call assert_notmatch('E609:', s:observations[-1].messages)
    call s:Query('g', 'Beta', [['beta.c', 1]])
    call s:Query(field, '[', [])
    call assert_match('E609:.*the query pattern cannot be used', s:observations[-1].messages)
    let caught = ''
    try
      execute 'cs find ' . field . ' ['
    catch
      let caught = v:exception
    endtry
    call assert_match('E609:.*the query pattern cannot be used', caught)
    call s:Query('g', 'Alpha', [['alpha.c', 1]])
  endfor
  call s:Query('a', 'Assigned', [['alpha.c', 4], ['beta.c', 6]])
  call s:Query('a', '^Assigned$', [['alpha.c', 4], ['beta.c', 6]])
  call s:Query('a', 'Alpha', [])
  call s:Query('s', 'Alpha', [['alpha.c', 1], ['beta.c', 2]])
  " Text uses live bytes; absolute pre-path keeps navigation stable after :cd.
  execute 'cd ' . fnameescape(s:root . '/elsewhere')
  call s:Query('t', 'Alpha', [['alpha.c', 1], ['beta.c', 2]])
  call s:Query('e', 'Alpha', [['alpha.c', 1], ['beta.c', 2]])
  call s:Query('e', '^int Alpha;$', [['alpha.c', 1]])
  call s:Query('e', '(Alpha|Beta)', [['alpha.c', 1], ['beta.c', 1], ['beta.c', 2]])
  call s:Query('t', 'Missing', [])
  call assert_match('E259:', s:observations[-1].error)
  call assert_notmatch('E609:', s:observations[-1].messages)
  " A file a text search cannot read is skipped; the rows come with a notice.
  call delete(s:root . '/beta.c')
  call s:Query('e', 'Alpha', [['alpha.c', 1]])
  call assert_match('E609:.*this answer may be incomplete', s:observations[-1].messages)
  call s:Query('g', 'Alpha', [['alpha.c', 1]])
  call s:Query('t', 'Alpha', [['alpha.c', 1]])
  call assert_match('E609:.*this answer may be incomplete', s:observations[-1].messages)
  call s:Query('t', 'Missing', [])
  call assert_match('E609:.*this answer may be incomplete', s:observations[-1].messages)
  call s:Query('g', 'Alpha', [['alpha.c', 1]])
  let caught = ''
  try
    cs find t Alpha
  catch
    let caught = v:exception
  endtry
  call assert_match('E609:.*this answer may be incomplete', caught)
  call add(s:observations, {'ordinary_text_error': caught})
  call s:Query('g', 'Alpha', [['alpha.c', 1]])
  call writefile(['// Repaired Alpha'], s:root . '/beta.c')
  call s:Query('e', 'Repaired.*Alpha$', [['beta.c', 1]])
  call s:Query('t', 'Alpha', [['alpha.c', 1], ['beta.c', 1]])
  call assert_notmatch('E609:', s:observations[-1].messages, 'repair clears the warning')
  call writefile(['int Beta;', 'int read_alpha(void) { return Alpha; }', '#include "changed.h"', '#include <unique.h>', 'int call_beta(int callback(int)) { Callee(); }', 'int assign(void) { Assigned += 2; }'], s:root . '/beta.c')
  call s:Query('i', '^shared\.h$', [['alpha.c', 2], ['beta.c', 3]])
  call s:Query('c', 'Callee', [['alpha.c', 3], ['beta.c', 5]])
  call s:Query('d', 'call_.*', [['alpha.c', 3], ['beta.c', 5]])
  call s:Query('a', 'Assigned', [['alpha.c', 4], ['beta.c', 6]])
  execute 'cd ' . fnameescape(s:root)
  " An explicit rebuild process publishes while the editor connection stays open.
  call writefile(readfile(s:root . '/alpha.c') + ['int Rebuilt;'], s:root . '/alpha.c')
  call s:Query('g', 'Rebuilt', [])
  call writefile(['r', 'q'], s:root . '/rebuild-input')
  let rebuild_job = job_start([$RIPSCOPE_BACKEND, '-dl', '-f', s:root . '/index'],
        \ {'in_io': 'file', 'in_name': s:root . '/rebuild-input',
        \  'out_io': 'file', 'out_name': s:root . '/rebuild-output',
        \  'err_io': 'file', 'err_name': s:root . '/rebuild-errors'})
  for retry in range(1000)
    if job_status(rebuild_job) !=# 'run'
      break
    endif
    sleep 10m
  endfor
  call assert_equal('dead', job_status(rebuild_job))
  call assert_equal(0, job_info(rebuild_job).exitval)
  call assert_equal([], readfile(s:root . '/rebuild-errors'))
  let rebuilt = join(readfile(s:root . '/rebuild-output', 'b'), "\n")
  call assert_equal(">> \n>> ", substitute(rebuilt, '\r', '', 'g'))
  call s:Query('g', 'Rebuilt', [['alpha.c', 5]])
  let old_pid = s:pid
  cs reset
  let connection = execute('cs show')
  let s:pid = matchstr(connection, '\n\s*0\s\+\zs\d\+')
  call assert_notequal(old_pid, s:pid, 'reset reconnects')
  call add(s:observations, {'connection': connection, 'rebuild': rebuilt})
  call s:Query('g', 'Rebuilt', [['alpha.c', 5]])
  call s:Disconnect()
  call s:Connect()
  call s:Query('g', 'Beta', [['beta.c', 1]])
  call s:Disconnect()
  " Vim checks that the index exists before it starts the backend (E563).
  for [index, message] in [['missing', 'E563:'],
        \ ['foreign', 'ripscope: .*foreign is not a ripscope index'],
        \ ['damaged', 'ripscope: damaged index']]
    let v:errmsg = ''
    let output = execute('silent! cs add ' . index)
    call assert_equal(0, cscope_connection(), index . ' must not connect')
    call assert_notequal('', v:errmsg, index . ' visible startup error')
    call assert_match(message, output . v:errmsg, index)
    call add(s:observations, {'startup': index, 'messages': output, 'error': v:errmsg})
    call s:Connect()
    call s:Query('g', 'Alpha', [['alpha.c', 1]])
    call s:Disconnect()
  endfor
  " A copied index searches the copy's tree: cscope's prepend path (-P) puts the
  " connection's directory before the relative names text searches read and show.
  execute 'cs add ' . fnameescape(s:root . '/copy/index') . ' ' . fnameescape(s:root . '/copy')
  let connection = execute('cs show')
  let s:pid = matchstr(connection, '\n\s*0\s\+\zs\d\+')
  call add(s:observations, {'connection': connection})
  call setqflist([], 'r')
  let output = execute('silent! cs find t Alpha')
  let found = map(getqflist(), 'fnamemodify(bufname(v:val.bufnr), ":p")')
  call assert_equal([s:root . '/copy/alpha.c', s:root . '/copy/beta.c'], found)
  call add(s:observations, {'request': 't Alpha (copy)', 'found': found, 'messages': output})
  let output = execute('cs find f alpha')
  call assert_equal(s:root . '/copy/alpha.c', expand('%:p'))
  call s:Disconnect()
  " Leave one live connection so qa! also exercises Vim's exit cleanup.
  call s:Connect()
catch
  call add(v:errors, v:exception . ' at ' . v:throwpoint)
endtry
call writefile([json_encode({'errors': v:errors, 'observations': s:observations})], 'vim.json')
if !empty(v:errors)
  cquit
endif
qa!
