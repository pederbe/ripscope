" One-step setup check, run by check-editor-setup in a fresh profile: open a C file,
" search, save an edit and search again, with the plugin loaded as Vim loads packages.
set nomore
let s:report = {'errors': [], 'searches': []}
function! s:Path(path) abort
  return substitute(fnamemodify(a:path, ':p'), '\\', '/', 'g')
endfunction
function! s:Search(pattern) abort
  let rows = []
  try
    execute 'cs find g ' . a:pattern
    for item in getqflist()
      call add(rows, {'file': s:Path(bufname(item.bufnr)), 'line': item.lnum, 'text': item.text})
    endfor
  catch /E259:/
  endtry
  let jump = {}
  if !empty(rows)
    cc 1
    let jump = {'file': s:Path(expand('%')), 'line': line('.'), 'text': getline('.')}
  endif
  call add(s:report.searches, {'pattern': a:pattern, 'rows': rows, 'jump': jump})
endfunction
try
  execute 'edit ' . fnameescape($RIPSCOPE_SETUP_FILE)
  let s:report.connection = execute('cs show')
  call s:Search('greet')
  if !empty($RIPSCOPE_SETUP_ABSENT)
    call s:Search($RIPSCOPE_SETUP_ABSENT)
  endif
  execute 'edit ' . fnameescape($RIPSCOPE_SETUP_FILE)
  call append('$', 'int fresh_symbol(void) { return 3; }')
  write
  call s:Search('fresh_symbol')
catch
  call add(s:report.errors, v:exception . ' at ' . v:throwpoint)
endtry
let s:report.messages = execute('messages')
call writefile([json_encode(s:report)], $RIPSCOPE_SETUP_REPORT)
qa!
