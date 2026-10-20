;;; ripscope-xcscope.el --- Pinned local Ripscope integration -*- lexical-binding: t; no-byte-compile: t -*-
;; SPDX-License-Identifier: GPL-2.0-or-later
;;; Commentary:
;; Load this file before xcscope, then call `ripscope-xcscope-setup', or `ripscope-setup'
;; from ripscope.el for projects found without configuration.
;; See README.md for installation, supported options and qualification limits.
;; It stays uncompiled, so that the dynamic bindings of xcscope's variables below take
;; effect even where a package manager compiles the package.
;;; Code:

(require 'cl-lib)
(require 'json)
(require 'subr-x)

(when (featurep 'xcscope)
  (error "Load ripscope-xcscope before any other xcscope copy"))
(let ((source (expand-file-name "vendor/xcscope.el"
                               (file-name-directory (or load-file-name buffer-file-name)))))
  (unless (equal (with-temp-buffer
                   (set-buffer-multibyte nil)
                   (insert-file-contents-literally source)
                   (secure-hash 'sha256 (current-buffer)))
                 "fc2700ccc37773c820b1c9f0b782c15cd64058fa009f606d8d24b515da2fac8e")
    (error "Pinned xcscope source hash mismatch"))
  ;; NOSUFFIX prevents an unrelated byte-compiled file from taking precedence.
  (load source nil t t))

(defgroup ripscope-xcscope nil "Local Ripscope navigation." :group 'tools)
(defcustom ripscope-xcscope-include-directories nil
  "Include search directories, relative to the project root, for index builds and
the sessions that refresh them."
  :type '(repeat directory))
(defcustom ripscope-xcscope-auto-refresh t
  "Refresh saved sources before normal queries.
Set to nil for manual updates through `cscope-index-files'."
  :type 'boolean)
(defvar ripscope-xcscope--backend nil)
(defvar ripscope-xcscope--enabled nil)
(defvar ripscope-xcscope--automatic nil
  "Non-nil when a search outside xcscope's own databases uses an automatic project.")
(defvar ripscope-xcscope--projects (make-hash-table :test #'equal)
  "Automatic projects, each a plist of :root, :index, :selection and :published, by root.")
(defvar ripscope-xcscope--located (make-hash-table :test #'equal)
  "The automatic project of each directory searched from.")
(defvar ripscope-xcscope--building nil)
(defvar ripscope-xcscope--sessions (make-hash-table :test #'equal)
  "Running Ripscope sessions, by executable, database directory and options.")

(cl-defstruct (ripscope-xcscope--session (:constructor ripscope-xcscope--session-new))
  "A long-running JSON session for one database, which answers one search at a time."
  key process
  (phase 'starting)
  (partial "")
  diagnostics search request rows notes error remaining)

(defun ripscope-xcscope--check (&optional build)
  "Check the local backend and, for BUILD, helper prerequisites."
  (when (file-remote-p default-directory)
    (user-error "Ripscope xcscope supports local projects only"))
  (when (and ripscope-xcscope--automatic
             (not (and ripscope-xcscope--backend (file-executable-p ripscope-xcscope--backend))))
    (let ((found (ripscope-xcscope--find-executable)))
      (unless found
        (user-error "Ripscope's executable is not installed; install it with M-x ripscope-install"))
      (ripscope-xcscope--use found)))
  (unless (and ripscope-xcscope--backend
               (file-executable-p ripscope-xcscope--backend)
               (equal cscope-program ripscope-xcscope--backend))
    (user-error "Configured Ripscope executable is missing or changed"))
  (when (and build (eq system-type 'gnu/linux))
    (dolist (program '("bash" "find" "sed"))
      (unless (executable-find program)
        (user-error "Missing Ripscope build prerequisite: %s" program)))))

(defun ripscope-xcscope--start-native (name buffer program &rest args)
  "Start local PROGRAM with ARGS, decoding its output as UTF-8.
A search instead goes to the database's long-running session."
  (if (and (equal name "cscope") (member "-L" args))
      (ripscope-xcscope--search buffer program args)
    (apply #'ripscope-xcscope--start-process name buffer program args)))

(defun ripscope-xcscope--argument-coding (arguments)
  "The coding that passes ARGUMENTS intact to a process, checking that it can."
  ;; Windows Emacs encodes command arguments using the process write coding.
  (let ((coding (if (eq system-type 'windows-nt)
                    (if (= w32-ansi-code-page 65001) 'utf-8-unix
                      (intern (format "cp%d" w32-ansi-code-page)))
                  'utf-8-unix)))
    (dolist (arg arguments)
      (unless (equal arg (decode-coding-string (encode-coding-string arg coding) coding))
        (user-error "Ripscope argument cannot be represented in %s: %s" coding arg)))
    coding))

(defun ripscope-xcscope--start-process (name buffer program &rest args)
  "Start local PROGRAM with ARGS, decoding its output as UTF-8."
  (ripscope-xcscope--check)
  (let* ((coding-system-for-read 'utf-8-unix)
         (coding-system-for-write (ripscope-xcscope--argument-coding (cons program args))))
    (apply #'start-process name buffer program args)))

;; xcscope runs each search as a process of its own, which it reads with its filter and
;; finishes with its sentinel. Here a search goes to a session that stays running, so
;; that it refreshes only after the sources change, and its answer is given to xcscope
;; through a stand-in process that does no I/O itself.

(defun ripscope-xcscope--search (buffer program args)
  "Ask the session for ARGS' database to run the search ARGS ends with.
Returns the stand-in process xcscope reads the answer from, into BUFFER."
  (pcase-let* ((`(,field ,pattern) (last args 2))
               ;; Unless a database of xcscope's own has been built there since.
               (project (and (not (file-exists-p (expand-file-name
                                                  (or (cadr (member "-f" args)) cscope-database-file))))
                             (gethash (directory-file-name default-directory)
                                      ripscope-xcscope--projects)))
               (options (if project
                            (ripscope-xcscope--project-options project (butlast args 2))
                          (butlast args 2)))
               (startup (cons "--json"
                              (mapcar (lambda (arg) (if (equal arg "-L") "-l" arg)) options)))
               (key (list program default-directory startup)))
    (when (string-match-p "[\0\r\n]" pattern)
      (user-error "A Ripscope search pattern is one line"))
    (let ((session (gethash key ripscope-xcscope--sessions))
          search)
      (unless (and session (process-live-p (ripscope-xcscope--session-process session)))
        (when (and project (not (plist-get project :published)))
          (ripscope-xcscope--index project program))
        (setq session (ripscope-xcscope--open key program startup)))
      (setq search (make-pipe-process :name "ripscope-search" :buffer buffer :noquery t))
      (setf (ripscope-xcscope--session-search session) search
            (ripscope-xcscope--session-request session)
            (concat (substring field 1) pattern "\n")
            (ripscope-xcscope--session-rows session) nil
            (ripscope-xcscope--session-notes session) nil
            (ripscope-xcscope--session-error session) nil)
      (when (eq (ripscope-xcscope--session-phase session) 'idle)
        (ripscope-xcscope--send session))
      search)))

(defun ripscope-xcscope--open (key program startup)
  "Start the session for KEY: PROGRAM with STARTUP options."
  (let* ((process-connection-type nil)
         (process (apply #'ripscope-xcscope--start-process
                         "ripscope-session" nil program startup))
         (session (ripscope-xcscope--session-new :key key :process process)))
    ;; Requests and frames are UTF-8 on every system.
    (set-process-coding-system process 'utf-8-unix 'utf-8-unix)
    (set-process-query-on-exit-flag process nil)
    (set-process-filter process (lambda (_ output) (ripscope-xcscope--receive session output)))
    (set-process-sentinel process (lambda (_ _) (ripscope-xcscope--ended session)))
    (puthash key session ripscope-xcscope--sessions)
    session))

(defun ripscope-xcscope-stop-sessions ()
  "Stop the running Ripscope sessions; the next search starts its own.
On Windows a session keeps its project directory and index in use, so an index build
stops them first, and this command lets the project be moved or removed."
  (interactive)
  (let (sessions)
    (maphash (lambda (_ session) (push session sessions)) ripscope-xcscope--sessions)
    (dolist (session sessions)
      (let ((process (ripscope-xcscope--session-process session)))
        (ripscope-xcscope--forget session)
        (set-process-sentinel process #'ignore)
        ;; Asked to quit, it closes the index and leaves the directory before it exits.
        (when (process-live-p process)
          (ignore-errors (process-send-string process "q\n"))
          (let ((deadline (+ (float-time) 2)))
            (while (and (process-live-p process) (< (float-time) deadline))
              (accept-process-output process 0.05))))
        (delete-process process)
        (setf (ripscope-xcscope--session-process session) nil)
        (when (ripscope-xcscope--session-search session)
          (ripscope-xcscope--finish session "the Ripscope session was stopped"))))))

(defun ripscope-xcscope--send (session)
  "Send SESSION's waiting search."
  (setf (ripscope-xcscope--session-phase session) 'result)
  (process-send-string (ripscope-xcscope--session-process session)
                       (ripscope-xcscope--session-request session)))

(defun ripscope-xcscope--json (line)
  "LINE's JSON value, with objects as alists and arrays as lists."
  (if (and (fboundp 'json-parse-string)
           (or (not (fboundp 'json-available-p)) (json-available-p)))
      (json-parse-string line :object-type 'alist :array-type 'list)
    ;; An Emacs built without native JSON.
    (let ((json-object-type 'alist) (json-array-type 'list) (json-key-type 'symbol))
      (json-read-from-string line))))

(defun ripscope-xcscope--receive (session output)
  "Take in SESSION's OUTPUT, a line at a time."
  (let ((text (concat (ripscope-xcscope--session-partial session) output))
        (start 0))
    (while (and (ripscope-xcscope--session-process session)
                (string-match "\n" text start))
      (let ((line (substring text start (match-beginning 0))))
        (setq start (match-end 0))
        (if (string-match-p "\\`[[{]" line)
            ;; Only the session's own frames are checked here: what xcscope does with
            ;; a complete answer runs outside, and its errors are its own.
            (when (condition-case problem
                      (ripscope-xcscope--frame session (ripscope-xcscope--json line))
                    (error
                     (ripscope-xcscope--fail
                      session (format "invalid response: %s" (error-message-string problem)))
                     nil))
              (ripscope-xcscope--finish session (ripscope-xcscope--session-error session)))
          ;; Anything else is a diagnostic, as on standard error.
          (push (concat line "\n") (ripscope-xcscope--session-diagnostics session)))))
    (setf (ripscope-xcscope--session-partial session) (substring text start))))

(defun ripscope-xcscope--frame (session value)
  "Take in SESSION's frame VALUE: whether it completes the answer."
  (let ((phase (ripscope-xcscope--session-phase session))
        (type (and (consp (car-safe value)) (alist-get 'type value))))
    (pcase type
      ("ready"
       (pcase phase
         ('starting
          (setf (ripscope-xcscope--session-phase session) 'idle)
          (when (ripscope-xcscope--session-search session) (ripscope-xcscope--send session))
          nil)
         ('ready
          (setf (ripscope-xcscope--session-phase session) 'idle)
          t)
         (_ (error "ready before the answer was complete"))))
      ("result"
       (let ((count (alist-get 'count value)))
         (unless (and (eq phase 'result) (natnump count)) (error "unexpected result header"))
         (setf (ripscope-xcscope--session-remaining session) count
               (ripscope-xcscope--session-phase session) (if (= count 0) 'ready 'rows))
         nil))
      ("error"
       (unless (eq phase 'result) (error "unexpected error frame"))
       (setf (ripscope-xcscope--session-error session) (alist-get 'message value)
             (ripscope-xcscope--session-phase session) 'ready)
       nil)
      ("warning"
       (unless (eq phase 'ready) (error "unexpected warning frame"))
       (push (alist-get 'message value) (ripscope-xcscope--session-notes session))
       nil)
      (_
       (unless (and (eq phase 'rows) (null type) (= (length value) 4))
         (error "unexpected row or frame"))
       (push (ripscope-xcscope--row value) (ripscope-xcscope--session-rows session))
       (when (= (cl-decf (ripscope-xcscope--session-remaining session)) 0)
         (setf (ripscope-xcscope--session-phase session) 'ready))
       nil))))

(defun ripscope-xcscope--text (value)
  "A row field VALUE: a string, or the bytes of one that is not UTF-8."
  (if (stringp value) value (decode-coding-string (apply #'unibyte-string value) 'utf-8)))

(defun ripscope-xcscope--row (row)
  "ROW, `[path, context, line, text]', as a line of cscope's One-shot output."
  (pcase-let ((`(,path ,context ,line ,text) row))
    (unless (and (stringp context) (natnump line)) (error "invalid row"))
    (format "%s %s %d %s\n" (ripscope-xcscope--text path) context line
            (ripscope-xcscope--text text))))

(defun ripscope-xcscope--finish (session failure)
  "Give SESSION's answer to the waiting search: its rows, or FAILURE."
  (let ((search (ripscope-xcscope--session-search session))
        (lines (append (reverse (ripscope-xcscope--session-diagnostics session))
                       (mapcar (lambda (note) (concat "ripscope: " note "\n"))
                               (reverse (ripscope-xcscope--session-notes session)))
                       (reverse (ripscope-xcscope--session-rows session)))))
    (setf (ripscope-xcscope--session-search session) nil
          (ripscope-xcscope--session-diagnostics session) nil)
    (when (process-live-p search)
      ;; Finished here, once: deleting the stand-in later must not finish it again.
      (let ((sentinel (process-sentinel search)))
        (set-process-sentinel search #'ignore)
        (unwind-protect
            (if failure
                (process-put search 'ripscope-failure failure)
              ;; In pieces, as a process's output arrives: xcscope matches its lines
              ;; with a regexp that cannot take a long answer at once.
              (let ((filter (process-filter search)))
                (dolist (piece (ripscope-xcscope--pieces lines))
                  (funcall filter search piece))))
          (when failure
            (process-put search 'ripscope-diagnostics (apply #'concat lines)))
          (funcall sentinel search "finished\n"))))))

(defun ripscope-xcscope--pieces (lines)
  "LINES joined into pieces of up to about 4096 characters."
  (let (pieces (piece nil) (size 0))
    (dolist (line lines)
      (when (and piece (> (+ size (length line)) 4096))
        (push (apply #'concat (nreverse piece)) pieces)
        (setq piece nil size 0))
      (push line piece)
      (setq size (+ size (length line))))
    (when piece (push (apply #'concat (nreverse piece)) pieces))
    (nreverse pieces)))

(defun ripscope-xcscope--fail (session reason)
  "Stop SESSION, which can no longer be trusted, failing its search with REASON."
  (let ((process (ripscope-xcscope--session-process session)))
    (setf (ripscope-xcscope--session-process session) nil)
    (when process
      (set-process-sentinel process #'ignore)
      (delete-process process))
    (ripscope-xcscope--forget session)
    (when (ripscope-xcscope--session-search session)
      (ripscope-xcscope--finish session reason))))

(defun ripscope-xcscope--forget (session)
  "Let the next search start a session of its own in SESSION's place."
  (let ((key (ripscope-xcscope--session-key session)))
    (when (eq (gethash key ripscope-xcscope--sessions) session)
      (remhash key ripscope-xcscope--sessions))))

(defun ripscope-xcscope--ended (session)
  "SESSION's process ended: its waiting search fails."
  (let ((process (ripscope-xcscope--session-process session)))
    (unless (process-live-p process)
      (ripscope-xcscope--forget session)
      (push (ripscope-xcscope--session-partial session)
            (ripscope-xcscope--session-diagnostics session))
      (setf (ripscope-xcscope--session-partial session) "")
      (when (ripscope-xcscope--session-search session)
        (ripscope-xcscope--finish session (format "%s, status %s" (process-status process)
                                                  (process-exit-status process)))))))

(defun ripscope-xcscope--options (_original)
  "Keep build-only options out of query invocations."
  (if ripscope-xcscope--building
      (append '("-k")
              (apply #'append
                     (mapcar (lambda (dir) (list "-I" dir))
                             ripscope-xcscope-include-directories)))
    (list (if ripscope-xcscope-auto-refresh "--auto-refresh" "--no-auto-refresh"))))

(defun ripscope-xcscope--start-builder (name buffer _program &rest args)
  "Start the pinned builder with failure-preserving shell semantics."
  (apply #'start-process name buffer (executable-find "bash")
         "-e" "-o" "pipefail" args))

(defun ripscope-xcscope--build (original directory header-text only-list)
  "Run ORIGINAL for DIRECTORY with strict helper and concurrency checks."
  (when (and cscope-unix-index-process (process-live-p cscope-unix-index-process))
    (user-error "A Ripscope build is already running"))
  (let ((default-directory (file-name-as-directory (expand-file-name directory))))
    (ripscope-xcscope--check t)
    (ripscope-xcscope-stop-sessions)
    (let ((ripscope-xcscope--building t))
      (if (eq system-type 'windows-nt)
          (progn
            (when only-list
              (user-error "Use cscope-index-files for native discovery, or maintain cscope.files yourself"))
            (let* ((process-connection-type nil)
                   (process (apply #'ripscope-xcscope--start-native "cscope-indexer" nil
                                   ripscope-xcscope--backend
                                   (append (list "-b" "-f" cscope-database-file)
                                           (when cscope-index-recursively '("-R"))
                                           (unless (equal cscope-index-file "cscope.files")
                                             (list "-i" cscope-index-file))
                                           (cscope-construct-custom-options-list)))))
              (setq cscope-indexing-status-string (or header-text "")
                    cscope-unix-index-process process)
              (set-process-filter process #'cscope-unix-index-files-filter)
              (set-process-sentinel process #'cscope-unix-index-files-sentinel)
              (set-process-query-on-exit-flag process nil)))
        (let ((cscope-start-file-process #'ripscope-xcscope--start-builder))
          (funcall original default-directory header-text only-list))))))

(defun ripscope-xcscope--databases (original &rest args)
  "Validate resolved database entries before xcscope accesses their files."
  (when (and (car args) (file-remote-p (car args)))
    (user-error "Ripscope xcscope supports local databases only"))
  (let ((entries (apply original args)))
    (dolist (entry entries)
      (when (and (consp entry) (stringp (car entry)) (file-remote-p (car entry)))
        (user-error "Ripscope xcscope supports local databases only")))
    entries))

(defun ripscope-xcscope--query (original &rest args)
  "Check local configuration before calling ORIGINAL with ARGS."
  (ripscope-xcscope--check)
  ;; xcscope refuses a second search only from its own buffer, and sets up the new one
  ;; over the state of the one still running.
  (maphash (lambda (_ session)
             (let ((waiting (ripscope-xcscope--session-search session)))
               (when (and waiting (process-live-p waiting))
                 (user-error "A Ripscope search is still running; %s stops it"
                             "M-x ripscope-xcscope-stop-sessions"))))
           ripscope-xcscope--sessions)
  (when (and cscope-initial-directory (file-remote-p cscope-initial-directory))
    (user-error "Ripscope xcscope supports local projects only"))
  (apply original args))

(defun ripscope-xcscope--query-sentinel (original process event)
  "Show a failed search as failed instead of claiming completion."
  (let ((failure (process-get process 'ripscope-failure)))
    (if (not failure)
        (funcall original process event)
      (with-current-buffer (process-buffer process)
        (let ((inhibit-read-only t))
          (goto-char cscope-last-output-point)
          (insert (or (process-get process 'ripscope-diagnostics) ""))
          (insert (format "\nRipscope search failed (%s).\n" failure))
          (setq cscope-process nil cscope-search-list nil cscope-first-match-point nil)
          (when cscope-start-directory
            (setq default-directory cscope-start-directory))
          (set-buffer-modified-p nil)))
      (message "Ripscope search failed; see %s for diagnostics" cscope-output-buffer-name)
      (delete-process process))))

;; Automatic projects: where xcscope finds no database of its own, a search goes to the
;; project and index that `ripscope --project' reports for its directory, so that every
;; editor finds the same ones.

(defun ripscope-xcscope--windows-p ()
  (eq system-type 'windows-nt))

(defun ripscope-xcscope--project (directory)
  "The automatic project of a search in DIRECTORY, as `ripscope --project' reports it:
a plist of :root, :index, :selection and :published."
  (let ((directory (file-name-as-directory (expand-file-name directory))))
    (or (gethash directory ripscope-xcscope--located)
        (puthash directory (ripscope-xcscope--locate directory) ripscope-xcscope--located))))

(defun ripscope-xcscope--locate (directory)
  "Ask the backend for the project of a search in DIRECTORY."
  (ripscope-xcscope--check)
  (with-temp-buffer
    (let* ((default-directory directory)
           (coding-system-for-read 'utf-8-unix)
           (coding-system-for-write
            (ripscope-xcscope--argument-coding (list ripscope-xcscope--backend directory)))
           (status (call-process ripscope-xcscope--backend nil t nil "--project" directory)))
      (unless (eql status 0)
        (user-error "Ripscope found no project for %s: %s" directory
                    (string-trim (buffer-string))))
      (let ((value (ripscope-xcscope--json (string-trim (buffer-string)))))
        (list :root (directory-file-name (expand-file-name (alist-get 'root value)))
              :index (alist-get 'index value)
              :selection (append (alist-get 'selection value) nil)
              :published (eq (alist-get 'published value) t))))))

(defun ripscope-xcscope--find-executable ()
  "The ripscope executable: on PATH, else where the installer puts it, else nil."
  (let* ((windows (ripscope-xcscope--windows-p))
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

(defun ripscope-xcscope--use (backend)
  "Search and build with BACKEND from now on."
  (setq ripscope-xcscope--backend backend cscope-program backend))

(defun ripscope-xcscope--explicit-p (directory info)
  "Whether INFO, xcscope's databases for DIRECTORY, names one of its own: through
`cscope-database-regexps', or a directory holding `cscope-database-file'."
  (or (cl-some (lambda (entry)
                 (let ((regexp (car-safe entry)))
                   (if (stringp regexp) (string-match-p regexp directory) (and regexp (symbolp regexp)))))
               cscope-database-regexps)
      (cl-some (lambda (item)
                 (and (consp item) (stringp (car item))
                      (or (file-regular-p (car item))
                          (file-exists-p (expand-file-name cscope-database-file (car item))))))
               info)))

(defun ripscope-xcscope--find-info (original top-directory)
  "Search an automatic project where xcscope finds no database of its own."
  (let ((info (funcall original top-directory))
        (directory (cscope-canonicalize-directory (or top-directory cscope-initial-directory))))
    (if (ripscope-xcscope--explicit-p directory info)
        info
      (let* ((project (ripscope-xcscope--project directory))
             (root (plist-get project :root)))
        (puthash root project ripscope-xcscope--projects)
        (list (list (file-name-as-directory root)))))))

(defun ripscope-xcscope--project-arguments (project)
  "PROJECT's selection arguments, with the include directories."
  (append (plist-get project :selection)
          (apply #'append (mapcar (lambda (dir) (list "-I" dir))
                                  ripscope-xcscope-include-directories))))

(defun ripscope-xcscope--project-options (project options)
  "OPTIONS with PROJECT's index in place of the database xcscope names, and its selection,
so that a session that finds no published index builds it as an index build would."
  (let ((rest options) result)
    (while rest
      (if (and (equal (car rest) "-f") (cdr rest))
          (setq result (cons (plist-get project :index) (cons "-f" result)) rest (cddr rest))
        (setq result (cons (car rest) result) rest (cdr rest))))
    (append (ripscope-xcscope--project-arguments project) (nreverse result))))

(defun ripscope-xcscope--index (project program)
  "Index PROJECT with PROGRAM, when a search there finds nothing published in its index."
  (let ((root (plist-get project :root))
        (args (append '("-b") (ripscope-xcscope--project-arguments project)
                      (list "-f" (plist-get project :index)))))
    (message "Ripscope: indexing %s for the first time..." (abbreviate-file-name root))
    (redisplay)
    (with-current-buffer (get-buffer-create "*cscope-indexing-buffer*")
      (let ((inhibit-read-only t)) (erase-buffer))
      (let* ((default-directory (file-name-as-directory root))
             (coding-system-for-read 'utf-8-unix)
             (coding-system-for-write (ripscope-xcscope--argument-coding (cons program args)))
             (status (apply #'call-process program nil t nil args)))
        (unless (eql status 0)
          (user-error "Ripscope could not index %s (%s); see *cscope-indexing-buffer*"
                      (abbreviate-file-name root) status))))
    (plist-put project :published t)
    (message "Ripscope: indexed %s" (abbreviate-file-name root))))

(defun ripscope-xcscope--enable (backend automatic)
  "Search with BACKEND, an executable found later when nil; AUTOMATIC enables
automatic projects."
  (unless (and (memq system-type '(gnu/linux windows-nt)) (not (version< emacs-version "27.1")))
    (user-error "Ripscope xcscope requires GNU Emacs 27.1 or newer on Linux or Windows"))
  (when ripscope-xcscope--enabled
    (user-error "Ripscope xcscope is already configured; restart to change backend"))
  (when backend
    (unless (and (stringp backend) (file-name-absolute-p backend)
                 (not (file-remote-p backend)) (file-executable-p backend)
                 (not (file-directory-p backend))
                 (or (not (eq system-type 'windows-nt))
                     (string-suffix-p ".exe" (downcase backend))))
      (user-error "Ripscope backend must be an executable absolute local file (.exe on Windows)"))
    (unless automatic
      (let ((ripscope-xcscope--backend backend) (cscope-program backend))
        (ripscope-xcscope--check t)))
    (ripscope-xcscope--use backend))
  (setq ripscope-xcscope--enabled t ripscope-xcscope--automatic automatic
        cscope-option-do-not-update-database t cscope-use-relative-paths t
        cscope-start-file-process #'ripscope-xcscope--start-native)
  (advice-add 'cscope-construct-custom-options-list :around #'ripscope-xcscope--options)
  (advice-add 'cscope-unix-index-files-internal :around #'ripscope-xcscope--build)
  (advice-add 'cscope-call :around #'ripscope-xcscope--query)
  (advice-add 'cscope-find-info :around #'ripscope-xcscope--databases)
  (when automatic
    ;; Outside the remote-database check, which runs first.
    (advice-add 'cscope-find-info :around #'ripscope-xcscope--find-info))
  (advice-add 'cscope-process-sentinel :around #'ripscope-xcscope--query-sentinel)
  (cscope-setup))

(defun ripscope-xcscope-setup (backend)
  "Use absolute local BACKEND for both xcscope builds and queries.
This selects one backend for the Emacs session; restart to change it."
  (ripscope-xcscope--enable backend nil))

(provide 'ripscope-xcscope)
;;; ripscope-xcscope.el ends here
