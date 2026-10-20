" Delivered configuration check; invoked by the check-vim example with --workflow.
set nomore
set autoread
let s:root = substitute(getcwd(), '\\', '/', 'g')
let s:shell = &shell
let s:shellslash = has('win32') ? &shellslash : 0
let s:observations = []
let g:ripscope = {'backend': s:root . (has('win32') ? '/backend.exe' : '/backend'), 'root': s:root, 'index': s:root . '/workflow.idx'}
let s:auto_refresh = $RIPSCOPE_AUTO_REFRESH !=# 'manual'
if !s:auto_refresh
  let g:ripscope.auto_refresh = 0
endif
if $RIPSCOPE_SELECTION ==# 'list'
  let g:ripscope.file_list = 'selection.txt'
else
  let g:ripscope.sources = ['workflow.c']
endif
if has('win32') && $RIPSCOPE_SELECTION ==# 'list'
  for key in ['backend', 'root', 'index']
    let g:ripscope[key] = substitute(g:ripscope[key], '/', '\\', 'g')
  endfor
endif
function! s:Failure() abort
  let failure = ''
  try
    RipscopeBuild
  catch
    let failure = v:exception
  endtry
  call assert_match('ripscope: build failed', failure)
  call add(s:observations, {'build_failure': failure})
endfunction
function! s:Connection() abort
  let connection = execute('cs show')
  call add(s:observations, {'connection': connection})
  let pid = matchstr(connection, '\n\s*0\s\+\zs\d\+')
  if !has('win32') && filereadable('/proc/' . pid . '/cmdline')
    call add(s:observations, {'pid': pid, 'argv': split(join(readfile('/proc/' . pid . '/cmdline', 'b'), "\n"), "\n")})
  endif
  return pid
endfunction
function! s:Navigate(field, pattern, file, line, shown, pid) abort
  execute 'cs find ' . a:field . ' ' . a:pattern
  call assert_equal(1, len(getqflist()))
  call assert_match(a:shown, getqflist()[0].text)
  checktime
  cc 1
  call assert_equal(s:root . '/' . a:file, substitute(expand('%:p'), '\\', '/', 'g'))
  call assert_equal(a:line, line('.'))
  call assert_match(a:shown, getline('.'))
  call assert_equal(a:pid, s:Connection(), 'automatic Refresh keeps the connection')
endfunction
function! s:AutomaticChanges(pid) abort
  call writefile(['int HeaderOld;'], s:root . '/workflow.h')
  call setline(1, ['#include "workflow.h"', 'int Saved;', 'void changed(void) { target(); }'])
  write
  call s:Navigate('g', 'Saved', 'workflow.c', 2, 'Saved', a:pid)
  call s:Navigate('c', 'target', 'workflow.c', 3, 'changed', a:pid)
  call writefile(['#include "workflow.h"', 'int Saved;', 'void changed(void) { target(); }', 'int Outside;'], s:root . '/workflow.c')
  call s:Navigate('g', 'Outside', 'workflow.c', 4, 'Outside', a:pid)
  call writefile(['int HeaderFresh;'], s:root . '/workflow.h')
  call s:Navigate('g', 'HeaderFresh', 'workflow.h', 1, 'HeaderFresh', a:pid)
  call writefile(['int Added;'], s:root . '/added.c')
  if $RIPSCOPE_SELECTION ==# 'list'
    call delete(s:root . '/selection.txt')
    let uncertain = execute('silent! cs find g Saved')
    call assert_match('E609:', uncertain, 'automatic Refresh failure must be visible')
    call assert_equal(1, len(getqflist()), 'last published rows remain navigable')
    cc 1
    call assert_equal(2, line('.'))
    call assert_match('Saved', getline('.'))
    call assert_equal(a:pid, s:Connection())
    call writefile(['workflow.c', 'added.c'], s:root . '/selection.txt')
    call s:Navigate('g', 'Added', 'added.c', 1, 'Added', a:pid)
    execute 'bwipeout! ' . fnameescape(s:root . '/added.c')
    call assert_equal(0, rename(s:root . '/added.c', s:root . '/renamed.c'))
    call writefile(['workflow.c', 'renamed.c'], s:root . '/selection.txt')
    call s:Navigate('g', 'Added', 'renamed.c', 1, 'Added', a:pid)
    execute 'bwipeout! ' . fnameescape(s:root . '/renamed.c')
    call assert_equal(0, delete(s:root . '/renamed.c'))
    call writefile(['workflow.c'], s:root . '/selection.txt')
  else
    call delete(s:root . '/added.c')
  endif
  let empty = execute('silent! cs find g Added')
  call assert_match('E259:', empty, 'explicit source lists stay fixed and removed list entries disappear')
  call assert_notmatch('E609:', empty)
  call assert_equal(a:pid, s:Connection())
  execute 'edit ' . fnameescape(s:root . '/workflow.c')
  call setline(1, ['int Workflow;', 'int Fresh;'])
  3,$delete _
  write
  call s:Navigate('g', 'Fresh', 'workflow.c', 2, 'Fresh', a:pid)
  execute 'silent! bwipeout! ' . fnameescape(s:root . '/workflow.h')
  call delete(s:root . '/workflow.h')
endfunction
function! s:InterruptBuild(timer) abort
  for job in job_info()
    let info = job_info(job)
    if info.status ==# 'run' && index(info.cmd, '-bk') >= 0
      let sid = getscriptinfo({'name': 'ripscope.vim$'})[0].sid
      let vars = getscriptinfo({'sid': sid})[0].variables
      call add(s:observations, {'interrupted_pid': info.process, 'output_file': vars.build_log, 'state': info.status})
      call interrupt()
      return
    endif
  endfor
  call add(v:errors, 'no running build found for interruption')
endfunction

try
  let valid = deepcopy(g:ripscope)
  for key in ['backend', 'root', 'index']
    let rejected = [valid[key] . "'", valid[key] . ';']
    if has('win32')
      call extend(rejected, ['C:relative', '/no-drive/path', '//server/share/index', 'C:/space dir/index', 'C:/bad&path'])
    endif
    for path in rejected
      let g:ripscope = deepcopy(valid)
      let g:ripscope[key] = path
      let failure = ''
      try
        execute 'source ' . fnameescape($RIPSCOPE_CONFIG)
      catch
        let failure = v:exception
      endtry
      call assert_match('require absolute paths using only ASCII', failure)
      call assert_false(filereadable(valid.index), 'reject before building')
      call assert_false(cscope_connection(), 'reject before connecting')
    endfor
  endfor
  let g:ripscope = valid
  execute 'source ' . fnameescape($RIPSCOPE_CONFIG)
  call assert_equal(s:shell, &shell)
  if has('win32')
    call assert_equal(s:shellslash, &shellslash)
    call rename(s:root . '/backend.exe', s:root . '/backend-away.exe')
    call s:Failure()
    call assert_match('failed to start', s:observations[-1].build_failure)
    call assert_false(cscope_connection())
    call rename(s:root . '/backend-away.exe', s:root . '/backend.exe')
    call writefile(repeat(['int Slow;'], 20000), s:root . '/workflow.c')
    let timer = timer_start(1, function('s:InterruptBuild'))
    let failure = ''
    try
      RipscopeBuild
    catch
      let failure = v:exception
    endtry
    call timer_stop(timer)
    call assert_match('build interrupted', failure)
    call assert_false(cscope_connection(), 'interruption does not connect')
    let script = getscriptinfo({'name': 'ripscope.vim$'})[0]
    let variables = getscriptinfo({'sid': script.sid})[0].variables
    call assert_false(has_key(variables, 'build_job'), 'interrupted job released before retry')
    call assert_false(has_key(variables, 'build_log'), 'interrupted output removed before retry')
    call add(s:observations, {'interruption_error': failure})
    call writefile(['int Workflow;'], s:root . '/workflow.c')
  endif
  execute 'cd ' . fnameescape(s:root . '/elsewhere')
  RipscopeBuild
  call assert_equal(s:shell, &shell)
  if has('win32')
    call assert_equal(s:shellslash, &shellslash)
  endif
  let pid = s:Connection()
  call assert_notequal('', pid)
  cs find g Workflow
  cc 1
  call assert_equal(s:root . '/workflow.c', substitute(expand('%:p'), '\\', '/', 'g'))
  call assert_equal(1, line('.'))
  if s:auto_refresh
    call s:AutomaticChanges(pid)
  else
  if has('win32')
    " Missing selected input is a real nonzero backend update, not a stub exit.
    call delete(s:root . '/workflow.c')
    call s:Failure()
    call assert_match('build failed (exit', s:observations[-1].build_failure)
  else
    " A failing executable must expose its output without reconnecting.
    call writefile(['#!/bin/sh', 'echo deliberate-backend-failure >&2', 'exit 23'], s:root . '/backend')
    call s:Failure()
    call assert_match('deliberate-backend-failure', s:observations[-1].build_failure)
    call assert_equal(pid, s:Connection())
    call writefile(['#!/bin/sh', 'exec ' . shellescape($RIPSCOPE_BACKEND) . ' "$@"'], s:root . '/backend')
  endif
  call writefile(['int Workflow;', 'int Fresh;'], s:root . '/workflow.c')
  if !has('win32')
    call system('chmod a-w ' . join(map(['', '-wal', '-shm'], 'shellescape(s:root . "/workflow.idx" . v:val)')))
    call s:Failure()
  endif
  cs find g Workflow
  call assert_equal(pid, s:Connection())
  let empty = execute('silent! cs find g Fresh')
  call assert_match('E259:', empty)
  call assert_notmatch('E609:', empty)
  if !has('win32')
    call system('chmod u+w ' . join(map(['', '-wal', '-shm'], 'shellescape(s:root . "/workflow.idx" . v:val)')))
  endif
  RipscopeBuild
  call assert_equal(pid, s:Connection())
  checktime
  cs find g Fresh
  cc 1
  call assert_equal(s:root . '/workflow.c', substitute(expand('%:p'), '\\', '/', 'g'))
  call assert_equal(2, line('.'))
  endif
  RipscopeDisconnect
  if !has('win32')
    call assert_false(isdirectory('/proc/' . pid))
  endif
  RipscopeConnect
  let restarted = s:Connection()
  call assert_notequal(pid, restarted)
  checktime
  cs find g Fresh
  cc 1
  call assert_equal(2, line('.'))
  RipscopeDisconnect
  " An unrelated connection must survive without changing its global settings.
  let &cscopeprg = $RIPSCOPE_BACKEND . ' --vim-errors'
  execute 'cs add ' . fnameescape(s:root . '/index') . ' ' . fnameescape(s:root)
  let unrelated = s:Connection()
  let oldprg = &cscopeprg
  let failure = ''
  try
    RipscopeConnect
  catch
    let failure = v:exception
  endtry
  call assert_match('unrelated cscope connection', failure)
  call assert_equal(oldprg, &cscopeprg)
  call assert_equal(unrelated, s:Connection())
  cs kill 0
catch
  call add(v:errors, v:exception . ' at ' . v:throwpoint)
finally
  if !has('win32')
    call system('chmod u+w ' . join(map(['', '-wal', '-shm'], 'shellescape(s:root . "/workflow.idx" . v:val)')))
  endif
  call writefile([json_encode({'errors': v:errors, 'observations': s:observations})], s:root . '/workflow.json')
endtry
if !empty(v:errors)
  cquit
endif
qa!
