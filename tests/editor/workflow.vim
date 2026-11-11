" Delivered configuration check; invoked by the check-vim example with --workflow.
set nomore
set autoread
let s:root = substitute(getcwd(), '\\', '/', 'g')
let s:shell = &shell
let s:shellslash = has('win32') ? &shellslash : 0
let s:observations = []
let g:ripscope = {'backend': s:root . (has('win32') ? '/backend.exe' : '/backend')}
if has('win32') && $RIPSCOPE_SELECTION ==# 'list'
  for key in ['backend']
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
  call assert_match('ripscope:', failure)
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
    call delete(s:root . '/cscope.files')
    let uncertain = execute('silent! cs find g Saved')
    call assert_match('E609:', uncertain, 'automatic Refresh failure must be visible')
    call assert_equal(1, len(getqflist()), 'last published rows remain navigable')
    cc 1
    call assert_equal(2, line('.'))
    call assert_match('Saved', getline('.'))
    call assert_equal(a:pid, s:Connection())
    call writefile(['workflow.c', 'added.c'], s:root . '/cscope.files')
    call s:Navigate('g', 'Added', 'added.c', 1, 'Added', a:pid)
    execute 'bwipeout! ' . fnameescape(s:root . '/added.c')
    call assert_equal(0, rename(s:root . '/added.c', s:root . '/renamed.c'))
    call writefile(['workflow.c', 'renamed.c'], s:root . '/cscope.files')
    call s:Navigate('g', 'Added', 'renamed.c', 1, 'Added', a:pid)
    execute 'bwipeout! ' . fnameescape(s:root . '/renamed.c')
    call assert_equal(0, delete(s:root . '/renamed.c'))
    call writefile(['workflow.c'], s:root . '/cscope.files')
  else
    call s:Navigate('g', 'Added', 'added.c', 1, 'Added', a:pid)
    call delete(s:root . '/added.c')
  endif
  let empty = execute('silent! cs find g Added')
  call assert_match('E259:', empty, 'removed files and list entries disappear')
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
    if info.status ==# 'run' && index(info.cmd, '-b') >= 0
      let sid = getscriptinfo({'name': 'ripscope.vim$'})[0].sid
      let vars = getscriptinfo({'sid': sid})[0].variables
      call add(s:observations, {'interrupted_pid': info.process, 'output_file': vars.job_log, 'state': info.status})
      call timer_stop(a:timer)
      call interrupt()
      return
    endif
  endfor
endfunction

try
  execute 'edit ' . fnameescape(s:root . '/workflow.c')
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
    let timer = timer_start(1, function('s:InterruptBuild'), {'repeat': -1})
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
    call assert_false(has_key(variables, 'job'), 'interrupted job released before retry')
    call assert_false(has_key(variables, 'job_log'), 'interrupted output removed before retry')
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
  call s:AutomaticChanges(pid)
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
  execute 'cs add ' . fnameescape($RIPSCOPE_WORKFLOW_INDEX) . ' ' . fnameescape(s:root)
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
  call writefile([json_encode({'errors': v:errors, 'observations': s:observations})], s:root . '/workflow.json')
endtry
if !empty(v:errors)
  cquit
endif
qa!
