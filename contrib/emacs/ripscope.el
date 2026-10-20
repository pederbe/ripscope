;;; ripscope.el --- Fast cscope searches for C, through xref -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Peder Bergan

;; Author: Peder Bergan
;; SPDX-License-Identifier: MIT OR Apache-2.0
;; Version: 0.2.0
;; Package-Requires: ((emacs "28.1"))
;; Keywords: c, languages, tools
;; URL: https://github.com/pederbe/ripscope

;; This file is not part of GNU Emacs.

;; This program is free software, licensed under either of the Apache License,
;; Version 2.0 or the MIT license, at your option.  See LICENSE-APACHE and
;; LICENSE-MIT in the repository, or
;; <https://www.apache.org/licenses/LICENSE-2.0> and
;; <https://opensource.org/license/mit>.

;;; Commentary:

;; Ripscope finds definitions, callers, called functions, assignments,
;; text and files in C projects, as cscope does.  After `ripscope-setup',
;; `xref-find-definitions' (M-.) finds a definition and
;; `xref-find-references' (M-?) the uses of a symbol in any C buffer,
;; with no configuration: the first search in a project indexes it into
;; your cache directory, and later searches see your saved edits.
;; Results open in the usual xref buffer.
;;
;; All nine searches are commands, such as `ripscope-find-callers'.
;; `ripscope-command-map' holds them under xcscope's keys, for a prefix
;; of your choice:
;;
;;   (define-key ripscope-mode-map (kbd "C-c s") 'ripscope-command-map)
;;
;; With use-package on Emacs 30 or newer, in one line:
;;
;;   (use-package ripscope
;;     :vc (:url "https://github.com/pederbe/ripscope" :branch "release"
;;          :lisp-dir "contrib/emacs" :rev :newest)
;;     :config (ripscope-setup))
;;
;; Searches need the ripscope executable.  Installed from the repository,
;; `ripscope-setup' installs the matching one; otherwise run the one-line
;; installer from https://ripscope.com.  See README.md beside this file.

;;; Code:

(require 'cl-lib)
(require 'json)
(require 'subr-x)
(require 'xref)

(defgroup ripscope nil
  "Search C projects with Ripscope."
  :group 'tools
  :prefix "ripscope-"
  :link '(url-link "https://ripscope.com"))

(defcustom ripscope-executable nil
  "The ripscope executable, or nil to find it.
When nil, searches use the ripscope on the variable `exec-path', else
the one Ripscope's installer puts in ~/.local/bin."
  :type '(choice (const :tag "Find it" nil) file))

(defcustom ripscope-include-directories nil
  "Directories to look for included headers in, relative to the project."
  :type '(repeat directory))

(defcustom ripscope-auto-refresh t
  "Whether a search first brings the index up to date with your edits.
When nil, the index changes only when `ripscope-index-project' runs."
  :type 'boolean)

(defcustom ripscope-timeout 120
  "How many seconds a search waits for its answer before it fails."
  :type 'number)

;;;; Finding the executable and the project

(defconst ripscope--fields
  '((0 . "uses of") (1 . "definitions of") (2 . "functions called by")
    (3 . "callers of") (4 . "text") (6 . "lines matching") (7 . "files named")
    (8 . "files including") (9 . "assignments to"))
  "What each of Ripscope's searches finds, by its field number.")

(defvar ripscope--projects (make-hash-table :test #'equal)
  "The projects searched, by root and index.
Each is a plist of :root, :index, :selection and :published.")

(defvar ripscope--located (make-hash-table :test #'equal)
  "The project of each directory searched from.")

(defvar ripscope--recent nil
  "The project last searched, for a search from a buffer that visits no file.")

(defvar ripscope--sessions (make-hash-table :test #'equal)
  "The running sessions, by executable, project root and index.")

(defvar ripscope--history nil
  "The patterns searched for.")

(defun ripscope--windows-p ()
  "Whether Emacs runs on Windows."
  (eq system-type 'windows-nt))

(defun ripscope--find-executable ()
  "The ripscope executable: on the variable `exec-path', else where installed."
  (let* ((windows (ripscope--windows-p))
         (name (if windows "ripscope.exe" "ripscope"))
         (found (executable-find "ripscope"))
         (home (or (and windows (getenv "USERPROFILE")) "~")))
    (if (and found (or (not windows) (string-suffix-p ".exe" (downcase found))))
        (expand-file-name found)
      (cl-some (lambda (directory)
                 (when (and directory (not (string-empty-p directory)))
                   (let ((candidate (expand-file-name name directory)))
                     (and (file-executable-p candidate) (not (file-directory-p candidate))
                          candidate))))
               (list (getenv "RIPSCOPE_INSTALL_DIR") (expand-file-name ".local/bin" home))))))

(defun ripscope--executable ()
  "The executable searches run, or a `user-error' saying how to install one."
  (let ((found (if ripscope-executable
                   (expand-file-name ripscope-executable)
                 (ripscope--find-executable))))
    (cond
     ((not found)
      (user-error "Ripscope is not installed, so run M-x ripscope-install, %s"
                  "or the one-line installer from https://ripscope.com"))
     ((not (and (file-executable-p found) (not (file-directory-p found))
                (or (not (ripscope--windows-p)) (string-suffix-p ".exe" (downcase found)))))
      (user-error "Ripscope cannot run %s, so set `ripscope-executable' to %s" found
                  (if (ripscope--windows-p) "the ripscope.exe file" "the ripscope executable")))
     (t found))))

(defun ripscope--argument-coding (arguments)
  "The coding that passes ARGUMENTS intact to a process, checking that it can."
  ;; Windows Emacs encodes command arguments with the process's write coding.
  (let ((coding (if (and (ripscope--windows-p) (boundp 'w32-ansi-code-page)
                         (/= (symbol-value 'w32-ansi-code-page) 65001))
                    (intern (format "cp%d" (symbol-value 'w32-ansi-code-page)))
                  'utf-8-unix)))
    (unless (coding-system-p coding)
      (user-error "Ripscope cannot pass arguments in Windows code page %s, which Emacs lacks"
                  coding))
    (dolist (argument arguments)
      (unless (equal argument (decode-coding-string (encode-coding-string argument coding)
                                                    coding))
        (user-error "Ripscope cannot pass %s to Windows in code page %s" argument coding)))
    coding))

(defun ripscope--call (executable buffer arguments)
  "Run EXECUTABLE with ARGUMENTS into BUFFER and return its exit status."
  (let ((coding-system-for-read 'utf-8-unix)
        (coding-system-for-write (ripscope--argument-coding (cons executable arguments))))
    (apply #'call-process executable nil buffer nil arguments)))

(defun ripscope--json (text)
  "TEXT's JSON value, with objects as alists and arrays as lists."
  (if (and (fboundp 'json-parse-string)
           (or (not (fboundp 'json-available-p)) (json-available-p)))
      (json-parse-string text :object-type 'alist :array-type 'list
                         :null-object nil :false-object nil)
    ;; An Emacs built without native JSON.
    (let ((json-object-type 'alist) (json-array-type 'list) (json-key-type 'symbol)
          (json-false nil))
      (json-read-from-string text))))

(defun ripscope--locate (executable path)
  "Ask EXECUTABLE for the project of PATH, a file or a directory."
  (with-temp-buffer
    (let ((status (ripscope--call executable t (list "--project" path))))
      (unless (eql status 0)
        (user-error "Ripscope found no project for %s: %s" path
                    (string-trim (replace-regexp-in-string "^ripscope: " "" (buffer-string)))))
      ;; The report is one line, and any warning beside it another.
      (goto-char (point-min))
      (unless (re-search-forward "^{.*}$" nil t)
        (user-error "Ripscope reported no project for %s: %s" path (string-trim (buffer-string))))
      (let* ((value (ripscope--json (match-string 0)))
             (root (directory-file-name (expand-file-name (alist-get 'root value))))
             (index (alist-get 'index value))
             (key (concat root "\0" index)))
        (or (gethash key ripscope--projects)
            (puthash key (list :root root :index index
                               :selection (alist-get 'selection value)
                               :published (eq (alist-get 'published value) t))
                     ripscope--projects))))))

(defun ripscope--project ()
  "The project of the current buffer's file or directory.
From the xref buffer, it is the project of the last search."
  (when (file-remote-p default-directory)
    (user-error "Ripscope searches local files only"))
  (let ((file buffer-file-name))
    (if (and (not file) ripscope--recent (derived-mode-p 'xref--xref-buffer-mode))
        ripscope--recent
      (let* ((path (expand-file-name (or file default-directory)))
             (directory (file-name-directory path)))
        (setq ripscope--recent
              (or (gethash directory ripscope--located)
                  (puthash directory (ripscope--locate (ripscope--executable) path)
                           ripscope--located)))))))

(defun ripscope--include-arguments ()
  "The include options of `ripscope-include-directories'."
  (mapcan (lambda (directory) (list "-I" directory)) ripscope-include-directories))

(defun ripscope--build (project)
  "Index PROJECT, showing the build's output in *ripscope-index*."
  (let* ((root (plist-get project :root))
         (executable (ripscope--executable))
         (buffer (get-buffer-create "*ripscope-index*"))
         (default-directory (file-name-as-directory root)))
    (ripscope--stop project)
    (with-current-buffer buffer
      (let ((inhibit-read-only t)) (erase-buffer)))
    (let ((status (ripscope--call executable buffer
                                  (append '("-b") (plist-get project :selection)
                                          (ripscope--include-arguments)
                                          (list "-f" (plist-get project :index))))))
      (unless (eql status 0)
        (user-error "Ripscope could not index %s (%s), and *ripscope-index* says why"
                    (abbreviate-file-name root) status)))
    (plist-put project :published t)))

;;;###autoload
(defun ripscope-index-project ()
  "Index the current file's project again.
Searches keep the index up to date by themselves, unless
`ripscope-auto-refresh' is nil."
  (interactive)
  (let ((project (ripscope--project)))
    (message "Ripscope: indexing %s..." (abbreviate-file-name (plist-get project :root)))
    (ripscope--build project)
    (message "Ripscope: indexed %s" (abbreviate-file-name (plist-get project :root)))))

;;;; Sessions

;; Each index has one long-running session, which answers one search at a time in
;; Ripscope's JSON interface and refreshes the index only after the sources change.

(cl-defstruct (ripscope--session (:constructor ripscope--session-create) (:copier nil))
  "A session for one index, and the search it is answering."
  key arguments process
  (phase 'starting)
  (partial "")
  request diagnostics rows notes error remaining done)

(defun ripscope--startup-arguments (project)
  "The arguments of a session for PROJECT, with the current settings."
  (append (list "--json" "-dl" (if ripscope-auto-refresh "--auto-refresh" "--no-auto-refresh"))
          (plist-get project :selection)
          (ripscope--include-arguments)
          (list "-f" (plist-get project :index) "-P" (plist-get project :root))))

(defun ripscope--session (project)
  "PROJECT's running session, started when it has none."
  (let* ((executable (ripscope--executable))
         (key (list executable (plist-get project :root) (plist-get project :index)))
         (arguments (ripscope--startup-arguments project))
         (session (gethash key ripscope--sessions)))
    (unless (and session (process-live-p (ripscope--session-process session))
                 (equal arguments (ripscope--session-arguments session)))
      (when session (ripscope--quit session))
      (setq session (ripscope--open key executable arguments (plist-get project :root))))
    session))

(defun ripscope--open (key executable arguments root)
  "Start the session for KEY: EXECUTABLE with ARGUMENTS, in ROOT."
  (let* ((default-directory (file-name-as-directory root))
         (coding-system-for-read 'utf-8-unix)
         (coding-system-for-write (ripscope--argument-coding (cons executable arguments)))
         (session (ripscope--session-create :key key :arguments arguments))
         (process (make-process
                   :name "ripscope" :command (cons executable arguments)
                   :connection-type 'pipe :noquery t
                   :filter (lambda (_ output) (ripscope--receive session output))
                   :sentinel (lambda (_ _) (ripscope--ended session)))))
    ;; Requests and frames are UTF-8 on every system.
    (set-process-coding-system process 'utf-8-unix 'utf-8-unix)
    (setf (ripscope--session-process session) process)
    (puthash key session ripscope--sessions)
    session))

(defun ripscope--send (session)
  "Send SESSION's waiting request."
  (setf (ripscope--session-phase session) 'result)
  (process-send-string (ripscope--session-process session) (ripscope--session-request session)))

(defun ripscope--receive (session output)
  "Take in SESSION's OUTPUT, a line at a time."
  (let ((text (concat (ripscope--session-partial session) output))
        (start 0))
    (while (and (not (ripscope--session-done session)) (string-match "\n" text start))
      (let ((line (substring text start (match-beginning 0))))
        (setq start (match-end 0))
        (if (string-match-p "\\`[[{]" line)
            (condition-case problem
                (ripscope--frame session (ripscope--json line))
              (error
               (ripscope--fail session (format "Ripscope sent a response it should not have (%s)"
                                               (error-message-string problem)))))
          ;; Anything else is a diagnostic, as on standard error.
          (push line (ripscope--session-diagnostics session)))))
    (setf (ripscope--session-partial session) (substring text start))))

(defun ripscope--frame (session value)
  "Take in VALUE, a frame of SESSION's answer."
  (let ((phase (ripscope--session-phase session))
        (type (and (consp (car-safe value)) (alist-get 'type value))))
    (pcase type
      ("ready"
       (pcase phase
         ('starting
          (setf (ripscope--session-phase session) 'idle)
          (when (ripscope--session-request session) (ripscope--send session)))
         ('ready
          (setf (ripscope--session-phase session) 'idle
                (ripscope--session-request session) nil
                (ripscope--session-done session) t))
         (_ (error "Ready before the answer was complete"))))
      ("result"
       (let ((count (alist-get 'count value)))
         (unless (and (eq phase 'result) (natnump count)) (error "Unexpected result header"))
         (setf (ripscope--session-remaining session) count
               (ripscope--session-phase session) (if (= count 0) 'ready 'rows))))
      ("error"
       (unless (and (eq phase 'result) (stringp (alist-get 'message value)))
         (error "Unexpected error frame"))
       (setf (ripscope--session-error session) (alist-get 'message value)
             (ripscope--session-phase session) 'ready))
      ("warning"
       (unless (and (eq phase 'ready) (stringp (alist-get 'message value)))
         (error "Unexpected warning frame"))
       (push (alist-get 'message value) (ripscope--session-notes session)))
      (_
       (unless (and (eq phase 'rows) (null type) (proper-list-p value) (= (length value) 4)
                    (stringp (nth 1 value)) (natnump (nth 2 value)) (> (nth 2 value) 0))
         (error "Unexpected row or frame"))
       (push value (ripscope--session-rows session))
       (when (= (cl-decf (ripscope--session-remaining session)) 0)
         (setf (ripscope--session-phase session) 'ready))))))

(defun ripscope--fail (session reason)
  "Stop SESSION, which can no longer be trusted, failing its search with REASON."
  (ripscope--forget session)
  (let ((process (ripscope--session-process session)))
    (set-process-sentinel process #'ignore)
    (delete-process process))
  (unless (ripscope--session-done session)
    (setf (ripscope--session-done session) (list 'failed reason))))

(defun ripscope--ended (session)
  "SESSION's process ended, failing its search."
  (let ((process (ripscope--session-process session)))
    (unless (process-live-p process)
      (ripscope--forget session)
      (unless (string-empty-p (ripscope--session-partial session))
        (push (ripscope--session-partial session) (ripscope--session-diagnostics session)))
      (unless (ripscope--session-done session)
        (setf (ripscope--session-done session)
              (list 'failed (format "Ripscope stopped unexpectedly (%s %s)"
                                    (process-status process) (process-exit-status process))))))))

(defun ripscope--forget (session)
  "Let the next search start a session in SESSION's place."
  (let ((key (ripscope--session-key session)))
    (when (eq (gethash key ripscope--sessions) session)
      (remhash key ripscope--sessions))))

(defun ripscope--quit (session)
  "Ask SESSION to quit, and stop it if it does not."
  (let ((process (ripscope--session-process session)))
    (ripscope--forget session)
    (set-process-sentinel process #'ignore)
    ;; Asked to quit, it closes the index and leaves the directory before it exits.
    (when (process-live-p process)
      (ignore-errors (process-send-string process "q\n"))
      (let ((deadline (+ (float-time) 2)))
        (while (and (process-live-p process) (< (float-time) deadline))
          (accept-process-output process 0.05))))
    (delete-process process)))

(defun ripscope--stop (project)
  "Stop PROJECT's sessions."
  (let (sessions)
    (maphash (lambda (key session)
               (when (equal (cdr key) (list (plist-get project :root) (plist-get project :index)))
                 (push session sessions)))
             ripscope--sessions)
    (mapc #'ripscope--quit sessions)))

;;;###autoload
(defun ripscope-stop-sessions ()
  "Stop the running Ripscope sessions.  The next search starts its own.
It also finds each project again.  On Windows a session keeps its project's
directory in use, so stop them before you move or remove a project."
  (interactive)
  (let (sessions)
    (maphash (lambda (_ session) (push session sessions)) ripscope--sessions)
    (mapc #'ripscope--quit sessions))
  (clrhash ripscope--located)
  (clrhash ripscope--projects)
  (setq ripscope--recent nil))

(defun ripscope--text (value)
  "A row field VALUE: a string, or the bytes of one that is not UTF-8."
  (if (stringp value) value (decode-coding-string (apply #'unibyte-string value) 'utf-8-unix)))

(defun ripscope--xref (root field row)
  "ROW, `[path, context, line, text]' from the search FIELD in ROOT, as an xref.
Its summary names the function a line is in, or a definition's line alone."
  (pcase-let* ((`(,path ,context ,line ,text) row)
               (path (ripscope--text path))
               (text (string-trim-right (ripscope--text text) "\r"))
               (summary (cond ((equal text "<unknown>") path)
                              ((or (= field 1) (member context '("<global>" "<unknown>"))) text)
                              (t (concat (propertize context 'face 'font-lock-function-name-face)
                                         ": " text)))))
    (xref-make summary (xref-make-file-location (expand-file-name path root) line 0))))

(defun ripscope--search (project field pattern)
  "Search PROJECT with the search FIELD for PATTERN.
Returns its results as xrefs, and shows any warnings that came with them."
  (when (string-match-p "[\0\r\n]" pattern)
    (user-error "Ripscope searches for one line at a time"))
  (when (string-empty-p pattern)
    (user-error "Enter something for Ripscope to search for"))
  (unless (plist-get project :published)
    (message "Ripscope: indexing %s for the first time..."
             (abbreviate-file-name (plist-get project :root)))
    (redisplay)
    (ripscope--build project))
  (let* ((session (ripscope--session project))
         (process (ripscope--session-process session))
         (deadline (+ (float-time) ripscope-timeout))
         finished)
    (setf (ripscope--session-request session) (format "%d%s\n" field pattern)
          (ripscope--session-rows session) nil
          (ripscope--session-notes session) nil
          (ripscope--session-error session) nil
          (ripscope--session-diagnostics session) nil
          (ripscope--session-done session) nil)
    (when (eq (ripscope--session-phase session) 'idle) (ripscope--send session))
    (unwind-protect
        (progn
          (while (and (not (ripscope--session-done session)) (< (float-time) deadline))
            (accept-process-output process 0.05))
          (setq finished (ripscope--session-done session)))
      ;; Interrupted, or out of time, its state is unknown.
      (unless finished (ripscope--fail session "interrupted")))
    (let ((done (ripscope--session-done session))
          (diagnostics (string-join (reverse (ripscope--session-diagnostics session)) "\n"))
          (notes (reverse (ripscope--session-notes session)))
          (failure (ripscope--session-error session)))
      (cond
       ((not finished)
        (error "Ripscope did not answer within %s seconds" ripscope-timeout))
       ((consp done)
        (error "%s%s" (cadr done) (if (string-empty-p diagnostics) "" (concat ": " diagnostics))))
       (failure (user-error "Ripscope could not search for %s: %s" pattern failure)))
      (let ((root (plist-get project :root)))
        (or (prog1 (mapcar (lambda (row) (ripscope--xref root field row))
                           (reverse (ripscope--session-rows session)))
              (when notes (message "Ripscope: %s" (string-join notes ", "))))
            (user-error "Ripscope found no %s %s%s" (alist-get field ripscope--fields) pattern
                        (if notes (concat " (" (string-join notes ", ") ")") "")))))))

;;;; Searches

(defun ripscope--read (prompt &optional file)
  "Read a pattern with PROMPT, offering the symbol at point.
For a FILE search, it offers the file name at point."
  (let ((default (if file (thing-at-point 'filename t) (thing-at-point 'symbol t))))
    (read-string (format-prompt prompt default) nil 'ripscope--history default)))

(defun ripscope--show (field pattern)
  "Show the results of the search FIELD for PATTERN in an xref buffer."
  (let ((project (ripscope--project)))
    (xref-show-xrefs (lambda () (ripscope--search project field pattern)) nil)))

;;;###autoload
(defun ripscope-find-symbol (symbol)
  "Find the uses of SYMBOL."
  (interactive (list (ripscope--read "Find symbol")))
  (ripscope--show 0 symbol))

;;;###autoload
(defun ripscope-find-definition (symbol)
  "Find the definitions of SYMBOL, going straight to a single one."
  (interactive (list (ripscope--read "Find definition")))
  (let ((xref-backend-functions (list #'ripscope--backend)))
    (xref-find-definitions symbol)))

;;;###autoload
(defun ripscope-find-callees (function)
  "Find the functions FUNCTION calls."
  (interactive (list (ripscope--read "Find functions called by")))
  (ripscope--show 2 function))

;;;###autoload
(defun ripscope-find-callers (function)
  "Find the functions that call FUNCTION."
  (interactive (list (ripscope--read "Find callers of")))
  (ripscope--show 3 function))

;;;###autoload
(defun ripscope-find-text (text)
  "Find the lines that hold TEXT."
  (interactive (list (ripscope--read "Find text")))
  (ripscope--show 4 text))

;;;###autoload
(defun ripscope-find-egrep (pattern)
  "Find the lines that match PATTERN, an extended regular expression."
  (interactive (list (ripscope--read "Find lines matching")))
  (ripscope--show 6 pattern))

;;;###autoload
(defun ripscope-find-file (name)
  "Find the files whose path contains NAME."
  (interactive (list (ripscope--read "Find file" t)))
  (ripscope--show 7 name))

;;;###autoload
(defun ripscope-find-includers (name)
  "Find the files that include the file NAME."
  (interactive (list (ripscope--read "Find files including" t)))
  (ripscope--show 8 name))

;;;###autoload
(defun ripscope-find-assignments (symbol)
  "Find the assignments to SYMBOL."
  (interactive (list (ripscope--read "Find assignments to")))
  (ripscope--show 9 symbol))

;;;; Xref and the minor mode

(defun ripscope--backend ()
  "Ripscope's xref backend, for a local buffer."
  (unless (file-remote-p default-directory) 'ripscope))

(cl-defmethod xref-backend-definitions ((_backend (eql ripscope)) identifier)
  "The definitions of IDENTIFIER, from Ripscope."
  (ripscope--search (ripscope--project) 1 identifier))

(cl-defmethod xref-backend-references ((_backend (eql ripscope)) identifier)
  "The uses of IDENTIFIER, from Ripscope."
  (ripscope--search (ripscope--project) 0 identifier))

(cl-defmethod xref-backend-identifier-completion-table ((_backend (eql ripscope)))
  "No completion: Ripscope searches for any name."
  nil)

(defvar ripscope-command-map
  (let ((map (make-sparse-keymap)))
    (define-key map "s" #'ripscope-find-symbol)
    (define-key map "g" #'ripscope-find-definition)
    (define-key map "d" #'ripscope-find-definition)
    (define-key map "C" #'ripscope-find-callees)
    (define-key map "c" #'ripscope-find-callers)
    (define-key map "t" #'ripscope-find-text)
    (define-key map "e" #'ripscope-find-egrep)
    (define-key map "f" #'ripscope-find-file)
    (define-key map "i" #'ripscope-find-includers)
    (define-key map "=" #'ripscope-find-assignments)
    (define-key map "I" #'ripscope-index-project)
    map)
  "Ripscope's searches, on the keys xcscope gives them.
Bind it in `ripscope-mode-map' to a prefix, such as the one xcscope uses.")
(fset 'ripscope-command-map ripscope-command-map)

(defvar ripscope-mode-map (make-sparse-keymap)
  "Keymap of `ripscope-mode'.")

;;;###autoload
(define-minor-mode ripscope-mode
  "Search C with Ripscope through xref.
\\[xref-find-definitions] then finds a definition, and \\[xref-find-references]
the uses of a symbol.  `ripscope-setup' turns it on in C buffers."
  :keymap ripscope-mode-map
  (if ripscope-mode
      (add-hook 'xref-backend-functions #'ripscope--backend nil t)
    (remove-hook 'xref-backend-functions #'ripscope--backend t)))

;;;; Setup and installation

(defconst ripscope--directory
  (file-name-directory (or load-file-name buffer-file-name))
  "The directory of this file.")

(defconst ripscope--repository
  (let ((root (expand-file-name "../.." ripscope--directory)))
    ;; Only a checkout, as plugin managers clone the repository, holds this file in
    ;; its contrib/emacs.  A copy the installer placed has no checkout around it.
    (and (file-equal-p (expand-file-name "contrib/emacs" root) ripscope--directory)
         (file-regular-p (expand-file-name "Cargo.toml" root))
         root))
  "The Ripscope checkout this copy belongs to, or nil when it is not in one.")

(defvar ripscope--installation nil
  "The running installer, if any.")

(defun ripscope--installer ()
  "This checkout's installer script, or nil when the copy is not in a checkout."
  (when ripscope--repository
    (let ((script (expand-file-name (if (ripscope--windows-p) "install.ps1" "install.sh")
                                    ripscope--repository)))
      (and (file-regular-p script) script))))

(defun ripscope--manifest ()
  "This checkout and the SHA-256 of its Cargo.toml, which names its version."
  (with-temp-buffer
    (insert-file-contents (expand-file-name "Cargo.toml" ripscope--repository))
    (concat ripscope--repository "\n" (secure-hash 'sha256 (current-buffer)))))

(defun ripscope--checked-file ()
  "Where the manifest the installer last checked the executable for is kept."
  (locate-user-emacs-file "ripscope-installer-checked"))

(defun ripscope--changed-p (found)
  "Whether the installer should look at FOUND, the executable or nil.
It should when FOUND is nil, or when the installer has not checked it
for this checkout's manifest, as after an upgrade of the package.
Starting Emacs otherwise runs no installer."
  (or (null found)
      (not (equal (ignore-errors
                    (with-temp-buffer
                      (insert-file-contents (ripscope--checked-file))
                      (buffer-string)))
                  (ripscope--manifest)))))

;;;###autoload
(defun ripscope-setup (&optional executable)
  "Search C buffers with Ripscope through xref.
It turns on `ripscope-mode' in C buffers, so that \\[xref-find-definitions]
and \\[xref-find-references] search with Ripscope.  A search's project is
the nearest directory upward holding cscope.files, else the nearest Git
repository, else the file's directory.  The first search there indexes
it into your cache directory.

EXECUTABLE, when given, sets `ripscope-executable'.  Otherwise, in a
copy installed from the repository, `ripscope-install' installs the
executable when it is missing or older than this copy."
  (when executable (setq ripscope-executable executable))
  (dolist (hook '(c-mode-hook c-ts-mode-hook))
    (add-hook hook #'ripscope-mode))
  (dolist (buffer (buffer-list))
    (with-current-buffer buffer
      (when (derived-mode-p 'c-mode 'c-ts-mode) (ripscope-mode))))
  (unless ripscope-executable
    (let ((found (ripscope--find-executable)))
      (cond
       ((ripscope--installer)
        (when (ripscope--changed-p found) (ripscope-install)))
       ((null found)
        ;; A message, not an error, so that the rest of the init file still loads.
        (message "Ripscope: the ripscope executable is not installed, so %s"
                 "run the one-line installer from https://ripscope.com"))))))

(defun ripscope--powershell ()
  "Windows PowerShell, which runs the Windows installer."
  (expand-file-name "System32/WindowsPowerShell/v1.0/powershell.exe"
                    (or (getenv "SystemRoot") "C:/Windows")))

;;;###autoload
(defun ripscope-install (&optional force)
  "Install the ripscope executable with Ripscope's verifying installer.
It installs this copy's version, but leaves an executable of that version
or newer alone, and one it did not install, unless FORCE, the prefix
argument, is given.  The installer runs in the background and reports in
the *ripscope-install* buffer."
  (interactive "P")
  (let ((windows (ripscope--windows-p))
        (script (ripscope--installer))
        (quiet (not (called-interactively-p 'any))))
    (cond
     ((process-live-p ripscope--installation)
      (message "Ripscope's installer is running already"))
     ((not script)
      (user-error "This copy of Ripscope has no installer, so %s"
                  "run the one-line installer from https://ripscope.com"))
     (t
      (let ((default-directory (file-name-as-directory ripscope--repository))
            (buffer (get-buffer-create "*ripscope-install*")))
        (with-current-buffer buffer
          (let ((inhibit-read-only t)) (erase-buffer)))
        (unless quiet (message "Ripscope: running the installer..."))
        (setq ripscope--installation
              (make-process
               :name "ripscope-install" :buffer buffer :noquery t
               :command (append (if windows
                                    (list (ripscope--powershell) "-NoProfile" "-NonInteractive"
                                          "-ExecutionPolicy" "Bypass" "-File" script "-Plugin")
                                  (list "sh" script "--plugin"))
                                (and force (list (if windows "-Force" "--force"))))
               :sentinel #'ripscope--installed))
        ;; Run at startup, it reports only an installation.
        (process-put ripscope--installation 'quiet quiet))))))

(defun ripscope--installed (process _event)
  "Report how PROCESS, the installer, went."
  (unless (process-live-p process)
    (let ((found (ripscope--find-executable))
          (said (let ((buffer (process-buffer process)))
                  (and (buffer-live-p buffer)
                       (with-current-buffer buffer
                         (save-excursion
                           (goto-char (point-max))
                           (and (re-search-backward
                                 "^\\(Installed [^\r\n]*\\|[^\r\n]* is installed: [^\r\n]*\\)" nil t)
                                (match-string 1))))))))
      (if (and found (eq (process-status process) 'exit) (= (process-exit-status process) 0))
          (progn
            (ignore-errors
              (let ((checked (ripscope--checked-file)))
                (make-directory (file-name-directory checked) t)
                (with-temp-file checked (insert (ripscope--manifest)))))
            (when (or (not (process-get process 'quiet))
                      (and said (string-prefix-p "Installed" said)))
              (message "Ripscope: %s" (or said (format "installed %s" found)))))
        (message "Ripscope: installation failed, and %s says why" (buffer-name (process-buffer process)))))))

(provide 'ripscope)
;;; ripscope.el ends here
