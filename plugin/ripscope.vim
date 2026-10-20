" Ripscope for Vim, through Vim's built-in cscope support.
"
" With no settings, opening a C file connects Vim to its project, so `:cs find` works
" at once. The first time, Ripscope indexes the project into your cache directory.
" Setting g:ripscope with a 'root' before this file loads chooses the project, index
" and sources yourself instead; `:RipscopeBuild` then builds the index.
" See docs/editors/vim.md.
if has('nvim')
  finish
endif
if exists('g:loaded_ripscope')
  " Loaded already, by Vim's startup or by another copy of this plugin.
  if exists('s:requested') && string(get(g:, 'ripscope', {})) !=# s:requested
    throw 'ripscope: configuration already loaded; start a new Vim to change projects'
  endif
  finish
endif
let s:settings = get(g:, 'ripscope', {})
let s:explicit = type(s:settings) == v:t_dict
      \ && !empty(filter(['root', 'index', 'sources', 'file_list'], 'has_key(s:settings, v:val)'))
let s:supported = (has('unix') || has('win32')) && has('cscope') && has('quickfix')
      \ && (!has('win32') || has('job'))
let s:path_pattern = has('win32') ? '^[A-Za-z]:/[A-Za-z0-9_./-]\+$' : '^/[A-Za-z0-9_./-]\+$'
let s:repository = fnamemodify(resolve(expand('<sfile>:p')), ':h:h')

if s:explicit
  if !s:supported
    throw 'ripscope: Vim with +cscope and +quickfix is required; Windows also requires +job'
  endif
  let s:config = deepcopy(s:settings)
  for s:key in ['backend', 'root', 'index']
    if has('win32') && type(get(s:config, s:key, 0)) == v:t_string
      let s:config[s:key] = substitute(s:config[s:key], '\\', '/', 'g')
    endif
    if type(get(s:config, s:key, 0)) != v:t_string || s:config[s:key] !~# s:path_pattern
      unlet s:config
      throw 'ripscope: backend, root and index require absolute paths using only ASCII letters, digits, slash, dot, underscore or hyphen (Windows drive colon allowed)'
    endif
  endfor
  if !isdirectory(s:config.root) || !executable(s:config.backend)
    unlet s:config
    throw 'ripscope: root directory and executable backend must exist'
  endif
  if empty(get(s:config, 'sources', [])) == empty(get(s:config, 'file_list', ''))
    unlet s:config
    throw 'ripscope: select exactly one of sources or file_list'
  endif
elseif !s:supported
  " Without cscope support there is nothing to connect; stay out of the way.
  finish
elseif type(s:settings) != v:t_dict
  let s:settings = {}
endif

let s:owned = 0
let s:added = []
let s:warned = {}

function! s:OwnConnection() abort
  return s:owned && !empty(s:added) && cscope_connection(4, s:added[0], s:added[1])
endfunction

function! s:Guard() abort
  " cscopeprg and query fan-out are global, so this profile owns the session.
  let rows = filter(split(execute('cs show'), "\n"), 'v:val =~# ''^\s*\d\+\s\+\d\+\s''')
  if len(rows) > (s:OwnConnection() ? 1 : 0)
    throw 'ripscope: unrelated cscope connection; use a separate Vim session'
  endif
endfunction

function! s:Automatic() abort
  return exists('s:config') && get(s:config, 'automatic', 0)
endfunction

" A path Vim can hand to its cscope program: on Windows, the short 8.3 form of one with
" spaces or other characters Vim cannot pass.
function! s:Usable(path) abort
  if a:path =~# s:path_pattern
    return a:path
  endif
  if has('win32')
    " Short names such as PROGRA~1 hold a tilde, which Vim passes as it is.
    let short = tr(fnamemodify(a:path, ':8'), '\', '/')
    if short =~# '^[A-Za-z]:/[A-Za-z0-9_./~-]\+$'
      return short
    endif
  endif
  throw 'ripscope: Vim''s cscope support cannot use ' . a:path
        \ . '; its paths may hold only ASCII letters, digits, slash, dot, underscore or hyphen'
        \ . (has('win32') ? ', and this one has no short 8.3 name' : '')
endfunction

" The executable, index and project root, as Vim's cscope support can take them. The
" index goes as it is: Vim turns a short name it is given for the index back into the
" long one, so an index path with spaces cannot be used.
function! s:Paths() abort
  if !s:Automatic()
    return [s:config.backend, s:config.index, s:config.root]
  endif
  if s:config.index !~# s:path_pattern
    throw 'ripscope: Vim''s cscope support cannot use the index ' . s:config.index
          \ . ', whose path holds a space or another character it cannot pass. Set the'
          \ . ' environment variable RIPSCOPE_CACHE_DIR to a folder without them, such as '
          \ . (has('win32') ? 'C:\ripscope-cache' : '/var/tmp/ripscope-cache')
  endif
  return [s:Usable(s:config.backend), s:config.index, s:Usable(s:config.root)]
endfunction

" Changes the directory without telling autocommands, returning the one before.
function! s:Enter(directory) abort
  noautocmd let previous = chdir(a:directory)
  return previous
endfunction

function! s:Includes() abort
  let arguments = []
  for directory in get(s:config, 'include_dirs', [])
    call extend(arguments, ['-I', directory])
  endfor
  return arguments
endfunction

function! s:Connect() abort
  call s:Guard()
  if s:OwnConnection()
    return
  endif
  let automatic = s:Automatic()
  " Without settings, cscopequickfix is set only as cscopeprg is taken over from Vim's
  " default, and only when it is empty, so a configured one stays.
  let default = &cscopeprg ==# 'cscope'
  let [backend, index, root] = s:Paths()
  " Paths are restricted above. Vim's direct argument parser keeps single quotes
  " in the executable name, so cscopeprg must use the validated path as it is.
  let s:program = backend . ' --vim-errors'
        \ . (get(s:config, 'auto_refresh', 1) ? ' --auto-refresh' : ' --no-auto-refresh')
  if automatic
    " A session that finds no published index builds it as an index build would.
    let s:program .= ' ' . join(s:config.selection + s:Includes(), ' ')
  endif
  let &cscopeprg = s:program
  if !automatic || (default && empty(&cscopequickfix))
    set cscopequickfix=s-,g-,d-,c-,f-,t-,e-,i-,a-
  endif
  " Vim starts its cscope program in its own directory: start it in the project, where
  " a session that builds the index must run.
  let previous = automatic ? s:Enter(s:config.root) : ''
  try
    execute 'cs add ' . fnameescape(index) . ' ' . fnameescape(root)
  finally
    if automatic && !empty(previous)
      call s:Enter(previous)
    endif
  endtry
  let s:added = [index, root]
  let s:owned = cscope_connection(4, index, root)
  if !s:owned
    throw 'ripscope: backend connection failed'
  endif
endfunction

function! s:Disconnect() abort
  call s:Guard()
  if s:OwnConnection()
    execute 'cs kill ' . fnameescape(s:added[0])
    let s:owned = 0
    let s:added = []
  endif
endfunction

" Runs ARGS in CWD with ENV added, returning the exit status and the combined output.
function! s:Run(args, cwd, env) abort
  if !has('win32')
    let prefix = empty(a:env) ? [] : ['env'] + map(items(a:env), 'v:val[0] . "=" . v:val[1]')
    let command = 'cd ' . shellescape(a:cwd) . ' && '
          \ . join(map(prefix + copy(a:args), 'shellescape(v:val)'), ' ')
    let output = system(command . ' 2>&1')
    return [v:shell_error, output]
  endif
  if exists('s:build_job') && job_status(s:build_job) ==# 'run'
    throw 'ripscope: previous build still running; output retained at ' . s:build_log
  endif
  if exists('s:build_log')
    call delete(s:build_log)
  endif
  let s:build_log = tempname()
  try
    let options = {'cwd': a:cwd, 'in_io': 'null', 'out_io': 'file', 'out_name': s:build_log,
          \ 'err_io': 'out', 'stoponexit': 'kill'}
    if !empty(a:env)
      let options.env = a:env
    endif
    let s:build_job = job_start(a:args, options)
    if job_status(s:build_job) ==# 'fail'
      throw 'ripscope: build failed to start: ' . a:args[0]
    endif
    while job_status(s:build_job) ==# 'run'
      sleep 10m
    endwhile
    return [job_info(s:build_job).exitval, join(readfile(s:build_log), "\n")]
  catch /^Vim:Interrupt$/
    throw 'ripscope: build interrupted; publication outcome must be checked before retry'
  finally
    if exists('s:build_job') && job_status(s:build_job) ==# 'run'
      call job_stop(s:build_job, 'kill')
      for retry in range(100)
        if job_status(s:build_job) !=# 'run'
          break
        endif
        sleep 10m
      endfor
    endif
    if exists('s:build_job') && job_status(s:build_job) ==# 'run'
      echomsg 'ripscope: build still running; output retained at ' . s:build_log
    else
      unlet! s:build_job
      for retry in range(100)
        if !filereadable(s:build_log) || delete(s:build_log) == 0
          unlet s:build_log
          break
        endif
        sleep 10m
      endfor
      if exists('s:build_log')
        echomsg 'ripscope: build output could not be removed: ' . s:build_log
      endif
    endif
  endtry
endfunction

function! s:Build() abort
  call s:Guard()
  if s:Automatic()
    let args = [s:config.backend, '-b'] + s:config.selection + s:Includes() + ['-f', s:config.index]
  else
    let args = [s:config.backend, '-bk', '-f', s:config.index]
    for directory in get(s:config, 'include_dirs', [])
      call extend(args, ['-I', directory])
    endfor
    if has_key(s:config, 'file_list') && !empty(s:config.file_list)
      call extend(args, ['-i', s:config.file_list])
    else
      call add(args, '--')
      call extend(args, s:config.sources)
    endif
  endif
  let [status, output] = s:Run(args, s:config.root, {})
  if status != 0 && s:Automatic() && output =~# 'another process'
    " Another editor's index build has held the index for a minute: wait once more.
    echomsg 'ripscope: another editor is indexing ' . s:config.root . '; waiting for it...'
    redraw
    let [status, output] = s:Run(args, s:config.root, {})
  endif
  if status != 0
    throw 'ripscope: build failed (exit ' . status . '): ' . output
  endif
  call s:Connect()
  " A build that cannot use some files succeeds and names each on a line of its own.
  let warnings = filter(split(output, "\n"), 'v:val =~# ''^ripscope: ''')
  return empty(warnings) ? '' : printf('ripscope: build completed with %d warning(s); first: %s',
        \ len(warnings), warnings[0][10:])
endfunction

function! s:BuildCommand() abort
  let warning = s:Build()
  echomsg empty(warning) ? 'ripscope: build completed' : warning
endfunction

" Automatic mode: the executable, then the project and its index, which `ripscope
" --project` reports, so that every editor finds the same ones.

function! s:Slashes(path) abort
  let path = has('win32') ? tr(a:path, '\', '/') : a:path
  return path =~# '^/$\|^\a:/$' ? path : substitute(path, '/\+$', '', '')
endfunction

function! s:Same(left, right) abort
  return has('win32') ? a:left ==? a:right : a:left ==# a:right
endfunction

function! s:InstallerExecutable() abort
  let directory = !empty($RIPSCOPE_INSTALL_DIR) ? $RIPSCOPE_INSTALL_DIR
        \ : (has('win32') ? $USERPROFILE : $HOME) . '/.local/bin'
  return s:Slashes(directory) . (has('win32') ? '/ripscope.exe' : '/ripscope')
endfunction

function! s:Executable() abort
  let configured = get(s:settings, 'backend', '')
  if !empty(configured)
    return s:Slashes(configured)
  endif
  let found = exepath('ripscope')
  if !empty(found) && (!has('win32') || found =~? '\.exe$')
    return s:Slashes(found)
  endif
  let name = has('win32') ? '/ripscope.exe' : '/ripscope'
  for directory in [$RIPSCOPE_INSTALL_DIR, (has('win32') ? $USERPROFILE : $HOME) . '/.local/bin']
    if !empty(directory) && executable(s:Slashes(directory) . name)
      return s:Slashes(directory) . name
    endif
  endfor
  return ''
endfunction

" Whether cscopeprg is still Vim's default or Ripscope's own, so Ripscope may set it.
function! s:OurProgram() abort
  return &cscopeprg ==# 'cscope' || &cscopeprg ==# get(s:, 'program', '')
endfunction

" The project of PATH, as `ripscope --project` reports it from Vim's directory.
" It runs without a shell where Vim has jobs, and only its JSON line is read, so that
" nothing a shell's startup files print gets in the way.
function! s:Locate(backend, path) abort
  let args = [a:backend, '--project', a:path]
  if has('job')
    let [out, err] = [tempname(), tempname()]
    let job = job_start(args, {'cwd': getcwd(), 'in_io': 'null', 'out_io': 'file',
          \ 'out_name': out, 'err_io': 'file', 'err_name': err})
    " Until it exits and Vim has written all it said.
    let waited = 0
    while job_status(job) ==# 'run' || (ch_status(job) ==# 'open' && waited < 200)
      sleep 10m
      let waited += job_status(job) ==# 'run' ? 0 : 1
    endwhile
    let status = job_info(job).exitval
    let lines = filereadable(out) ? readfile(out) : []
    let problem = filereadable(err) ? join(readfile(err), ' ') : ''
    call delete(out)
    call delete(err)
  else
    let lines = systemlist('cd ' . shellescape(getcwd()) . ' && '
          \ . join(map(copy(args), 'shellescape(v:val)'), ' '))
    let [status, problem] = [v:shell_error, join(lines, ' ')]
  endif
  let json = filter(copy(lines), 'v:val =~# ''^{''')
  if status != 0 || empty(json)
    throw 'ripscope: ' . (empty(problem) ? 'ripscope --project failed (exit ' . status . ')'
          \ : substitute(problem, '^ripscope: ', '', ''))
  endif
  return json_decode(json[-1])
endfunction

" Connects to the project of FILE, the connected project when FILE is empty. Its index
" is built first when nothing is published in it yet, or always when BUILD is set.
function! s:Connected(file, build) abort
  if empty(a:file) && !exists('s:config')
    throw 'ripscope: open a C file of the project first'
  endif
  if !s:OurProgram()
    throw 'ripscope: cscopeprg names another cscope program (' . &cscopeprg
          \ . '); Ripscope leaves it alone'
  endif
  let backend = s:Executable()
  if empty(backend)
    throw 'ripscope: the ripscope executable was not found; install it with :RipscopeInstall'
  endif
  let project = s:Locate(backend, empty(a:file) ? s:config.root : a:file)
  if exists('s:config') && !s:Same(s:config.root, project.root)
    call s:Disconnect()
  endif
  let s:config = {'automatic': 1, 'backend': backend, 'root': project.root,
        \ 'index': project.index, 'selection': project.selection,
        \ 'auto_refresh': get(s:settings, 'auto_refresh', 1),
        \ 'include_dirs': get(s:settings, 'include_dirs', [])}
  " Before indexing, so that a project Vim cannot connect to is not indexed for it.
  call s:Paths()
  if project.published && !a:build
    call s:Connect()
    return
  endif
  if !project.published
    echomsg 'ripscope: indexing ' . project.root . ' for the first time...'
    redraw
  endif
  let warning = s:Build()
  echomsg !empty(warning) ? warning : !project.published
        \ ? 'ripscope: indexed ' . project.root . '; search it with :cs find'
        \ : 'ripscope: build completed'
endfunction

" A C file was opened or first saved: connect to its project unless Vim has a cscope
" connection, or uses another cscope program, such as gtags-cscope.
function! s:Opened(file) abort
  if &buftype !=# '' || cscope_connection() || !s:OurProgram()
    return
  endif
  try
    call s:Connected(a:file, 0)
  catch /^ripscope:/
    if !has_key(s:warned, v:exception)
      let s:warned[v:exception] = 1
      echohl WarningMsg
      echomsg v:exception
      echohl None
    endif
  endtry
endfunction

" The version of the Ripscope repository this plugin came from, or ''.
function! s:CheckoutVersion() abort
  let manifest = s:repository . '/Cargo.toml'
  if !filereadable(manifest)
    return ''
  endif
  for line in readfile(manifest, '', 40)
    let number = matchstr(line, '^version = "\zs[^"]\+\ze"')
    if !empty(number)
      return number
    endif
  endfor
  return ''
endfunction

" Whether version OLD is older than NEW, by their numbers.
function! s:Older(old, new) abort
  let old = map(split(matchstr(a:old, '^[0-9.]*'), '\.'), 'str2nr(v:val)')
  let new = map(split(matchstr(a:new, '^[0-9.]*'), '\.'), 'str2nr(v:val)')
  for position in range(max([len(old), len(new)]))
    if get(old, position, 0) != get(new, position, 0)
      return get(old, position, 0) < get(new, position, 0)
    endif
  endfor
  return 0
endfunction

" Installs the executable of this checkout's version with the repository's installer,
" when none is found or the one found is older, or always when FORCE is set.
function! s:Install(force) abort
  let wanted = s:CheckoutVersion()
  let found = s:Executable()
  if !a:force && !empty(found)
    let [status, output] = s:Run([found, '--version'], s:repository, {})
    let have = matchstr(output, 'ripscope \zs\S\+')
    " Only an executable the installer put there is upgraded, not a system package's.
    let ours = s:Same(found, s:InstallerExecutable())
    if status == 0 && (empty(wanted) || !ours || !s:Older(have, wanted))
      echomsg 'ripscope: ripscope ' . have . ' is installed: ' . found
      return
    endif
  endif
  let script = s:repository . (has('win32') ? '/install.ps1' : '/install.sh')
  if !filereadable(script)
    throw 'ripscope: this copy has no installer; run the one-line installer from https://ripscope.com'
  endif
  let command = has('win32')
        \ ? [$SystemRoot . '/System32/WindowsPowerShell/v1.0/powershell.exe', '-NoProfile',
        \    '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script]
        \ : ['sh', script]
  echomsg 'ripscope: installing ripscope ' . wanted . '...'
  redraw
  " The plugin is here already, so the installer leaves the editors alone.
  let environment = {'RIPSCOPE_EDITORS': 'none'}
  if !empty(wanted)
    let environment.RIPSCOPE_VERSION = wanted
  endif
  let [status, output] = s:Run(command, s:repository, environment)
  if status != 0
    throw 'ripscope: installation failed (exit ' . status . '): ' . output
  endif
  let lines = filter(split(output, "\n"), 'v:val =~# ''^Installed''')
  echomsg empty(lines) ? 'ripscope: installed' : 'ripscope: ' . lines[-1]
endfunction

let s:requested = string(s:settings)
if s:explicit
  command! RipscopeBuild call <SID>BuildCommand()
  command! RipscopeConnect call <SID>Connect()
else
  command! RipscopeBuild call <SID>Connected(expand('%:p'), 1)
  command! RipscopeConnect call <SID>Connected(expand('%:p'), 0)
  augroup ripscope
    autocmd!
    " Not a new file's buffer: its project is known once it is saved.
    autocmd BufReadPost,BufWritePost *.c,*.h call <SID>Opened(expand('<afile>:p'))
  augroup END
endif
command! RipscopeDisconnect call <SID>Disconnect()
command! -bang RipscopeInstall call <SID>Install(<bang>0)
let g:loaded_ripscope = 1
