;;; emacs-workflow.el --- Real Emacs/Ripscope workflow tests -*- lexical-binding: t; no-byte-compile: t -*-
(require 'ert)
(require 'cl-lib)
(require 'json)
(require 'xref)
(require 'checkdoc)
(defvar ripscope-test-package (getenv "RIPSCOPE_EMACS_PACKAGE"))
(load ripscope-test-package nil t)
(defvar ripscope-test-backend
  (let ((raw (getenv "RIPSCOPE_EMACS_BACKEND")))
    (if (eq system-type 'windows-nt)
        ;; getenv has decoded the inherited bytes with the locale, which can differ
        ;; from Windows' ANSI code page.
        (decode-coding-string (if (multibyte-string-p raw)
                                  (encode-coding-string raw locale-coding-system)
                                raw)
                              (if (= w32-ansi-code-page 65001) 'utf-8
                                (intern (format "cp%d" w32-ansi-code-page))))
      raw)))
(ripscope-setup ripscope-test-backend)
(setq make-backup-files nil auto-save-default nil create-lockfiles nil vc-handled-backends nil)

(defun ripscope-test-pids ()
  "The process IDs of the running sessions."
  (let (pids)
    (maphash (lambda (_ session)
               (let ((process (ripscope--session-process session)))
                 (when (process-live-p process) (push (process-id process) pids))))
             ripscope--sessions)
    pids))

(defun ripscope-test-commands ()
  "The command lines of the running sessions."
  (let (commands)
    (maphash (lambda (_ session)
               (push (process-command (ripscope--session-process session)) commands))
             ripscope--sessions)
    commands))

(defun ripscope-test-source (root &optional file)
  "Show FILE, case.c by default, of ROOT in the selected window and make it current."
  (let ((buffer (find-file-noselect (expand-file-name (or file "case.c") root))))
    (delete-other-windows)
    (switch-to-buffer buffer)
    buffer))

(defun ripscope-test-rows (root field pattern)
  "The results of the search FIELD for PATTERN from case.c, as (FILE LINE SUMMARY)."
  (with-current-buffer (ripscope-test-source root)
    (mapcar (lambda (item)
              (let ((location (xref-item-location item)))
                (list (file-relative-name (xref-location-group location) root)
                      (xref-location-line location)
                      (substring-no-properties (xref-item-summary item)))))
            (ripscope--search (ripscope--project) field pattern))))

(defun ripscope-test-landed ()
  "The file and line the selected window shows."
  (with-current-buffer (window-buffer (selected-window))
    (list (buffer-file-name) (line-number-at-pos (window-point)))))

(defun ripscope-test-jump (root command pattern file line)
  "Run COMMAND for PATTERN from case.c, follow its first result, and check that it
lands on LINE of FILE."
  (save-current-buffer
    (ripscope-test-source root)
    (funcall command pattern)
    (let ((results (get-buffer "*xref*")))
      (when (and results (eq (window-buffer (selected-window)) results))
        (with-current-buffer results
          (goto-char (point-min))
          (xref-next-line-no-show)
          (xref-goto-xref))))
    (let ((landed (ripscope-test-landed)))
      (should (equal (file-truename (car landed)) (file-truename (expand-file-name file root))))
      (should (= (cadr landed) line)))))

(defun ripscope-test-write (file text)
  "Write TEXT to FILE as UTF-8 with LF endings."
  (let ((coding-system-for-write 'utf-8-unix))
    (write-region text nil file nil 'quiet)))

(defun ripscope-test-message (form)
  "The messages FORM, a function, shows."
  ;; Batch Emacs keeps no *Messages*.
  (let* ((shown nil)
         (original (symbol-function 'message)))
    (cl-letf (((symbol-function 'message)
               (lambda (format &rest args)
                 (when format (push (apply #'format-message format args) shown))
                 (apply original format args))))
      (funcall form))
    (string-join (nreverse shown) "\n")))

(defmacro ripscope-test-project (&rest body)
  "Run BODY in a new Git project ROOT, whose name holds spaces and accents."
  `(let* ((root (directory-file-name (make-temp-file "ripscope emacs & % é-" t)))
          (default-directory (file-name-as-directory root)))
     (unwind-protect
         (progn
           (make-directory ".git")
           (ripscope-test-write "case.h" "extern int value;\n")
           (ripscope-test-write "case.c" "#include \"case.h\"\nint value;\nint callee(void) { return value; }\nint caller(void) { value = 7; return callee(); }\n")
           ,@body)
       (dolist (buffer (buffer-list))
         (when (and (buffer-file-name buffer)
                    (string-prefix-p root (buffer-file-name buffer)))
           (with-current-buffer buffer (set-buffer-modified-p nil))
           (kill-buffer buffer)))
       (when (get-buffer "*xref*") (kill-buffer "*xref*"))
       ;; Windows keeps a running session's directory from being removed.
       (ripscope-stop-sessions)
       (delete-directory root t))))

(defun ripscope-test-published (root)
  "Whether the index `ripscope --project' names for ROOT is published."
  (with-temp-buffer
    (call-process ripscope-test-backend nil t nil "--project" root)
    (eq t (alist-get 'published (ripscope--json (buffer-string))))))

(ert-deftest ripscope-first-search-indexes-and-all-nine-searches-jump ()
  (ripscope-test-project
   (should-not (ripscope-test-published root))
   (dolist (case '((ripscope-find-symbol "value" "case.c" 2)
                   (ripscope-find-definition "callee" "case.c" 3)
                   (ripscope-find-callees "caller" "case.c" 4)
                   (ripscope-find-callers "callee" "case.c" 4)
                   (ripscope-find-text "value = 7" "case.c" 4)
                   (ripscope-find-egrep "value.*7" "case.c" 4)
                   (ripscope-find-file "case.c" "case.c" 1)
                   (ripscope-find-includers "case.h" "case.c" 1)
                   (ripscope-find-assignments "value" "case.c" 4)))
     (apply #'ripscope-test-jump root case))
   (should (ripscope-test-published root))
   ;; Every search went to the one session.
   (should (= (length (ripscope-test-pids)) 1))
   (should (equal (ripscope-test-rows root 0 "value")
                  '(("case.c" 2 "int value;") ("case.h" 1 "extern int value;")
                    ("case.c" 3 "callee: int callee(void ) { return value; }")
                    ("case.c" 4 "caller: int caller(void ) { value = 7; return callee(); }"))))
   (should (equal (ripscope-test-rows root 7 "case") '(("case.c" 1 "case.c") ("case.h" 1 "case.h"))))))

(ert-deftest ripscope-xref-keys-in-c-buffers ()
  (ripscope-test-project
   (with-current-buffer (ripscope-test-source root)
     (should (derived-mode-p 'c-mode))
     (should ripscope-mode)
     (should (eq (key-binding (kbd "M-.")) #'xref-find-definitions))
     (goto-char (point-min))
     (search-forward "return callee")
     (let ((xref-prompt-for-identifier nil))
       (call-interactively #'xref-find-definitions)))
   (should (equal (cadr (ripscope-test-landed)) 3))
   (with-current-buffer (ripscope-test-source root)
     (xref-find-references "value"))
   (with-current-buffer "*xref*"
     (goto-char (point-min))
     (let ((count 0))
       (while (let ((before (point))) (xref-next-line-no-show) (> (point) before))
         (cl-incf count))
       (should (= count 4))))
   ;; The optional keymap on xcscope's prefix.
   (define-key ripscope-mode-map (kbd "C-c s") 'ripscope-command-map)
   (unwind-protect
       (with-current-buffer (ripscope-test-source root)
         (should (eq (key-binding (kbd "C-c s c")) #'ripscope-find-callers))
         (should (eq (key-binding (kbd "C-c s =")) #'ripscope-find-assignments))
         (should (eq (key-binding (kbd "C-c s g")) #'ripscope-find-definition)))
     (define-key ripscope-mode-map (kbd "C-c s") nil))
   ;; Another language's buffer keeps its own backend.
   (with-current-buffer (find-file-noselect (expand-file-name "notes.txt" root))
     (should-not ripscope-mode))))

(ert-deftest ripscope-saved-and-external-changes-refresh-in-one-session ()
  (ripscope-test-project
   (ripscope-test-jump root #'ripscope-find-definition "callee" "case.c" 3)
   (let ((pid (ripscope-test-pids)))
     (with-current-buffer (ripscope-test-source root)
       (erase-buffer)
       (insert "#include \"case.h\"\nint Saved;\nvoid changed(void) { target(); }\n")
       (save-buffer))
     (ripscope-test-jump root #'ripscope-find-definition "Saved" "case.c" 2)
     (ripscope-test-jump root #'ripscope-find-callers "target" "case.c" 3)
     (ripscope-test-write "case.c" "#include \"case.h\"\nint Saved;\nvoid changed(void) { target(); }\nint Outside;\n")
     (with-current-buffer (get-file-buffer (expand-file-name "case.c" root)) (revert-buffer t t))
     (ripscope-test-jump root #'ripscope-find-definition "Outside" "case.c" 4)
     (ripscope-test-write "case.h" "int HeaderFresh;\n")
     (ripscope-test-jump root #'ripscope-find-definition "HeaderFresh" "case.h" 1)
     (make-directory "nested é")
     (ripscope-test-write "nested é/added file.c" "int Added;\n")
     (ripscope-test-jump root #'ripscope-find-definition "Added" "nested é/added file.c" 1)
     (rename-file "nested é/added file.c" "renamed.c")
     (ripscope-test-jump root #'ripscope-find-definition "Added" "renamed.c" 1)
     (delete-file "renamed.c")
     (should-error (ripscope-test-rows root 1 "Added") :type 'user-error)
     ;; One session answered every search, refreshing on its own.
     (should (equal (ripscope-test-pids) pid))
     (let ((command (car (ripscope-test-commands))))
       (should (member "--json" command))
       (should (member "-dl" command))
       (should (member "--auto-refresh" command)))
     ;; A stopped session gives way to a new one at the next search.
     (ripscope-stop-sessions)
     (should-not (ripscope-test-pids))
     (ripscope-test-jump root #'ripscope-find-definition "Saved" "case.c" 2)
     (should (= (length (ripscope-test-pids)) 1))
     (should-not (equal (ripscope-test-pids) pid)))))

(ert-deftest ripscope-manual-refresh-waits-for-an-index-build ()
  (ripscope-test-project
   (let ((ripscope-auto-refresh nil))
     (ripscope-test-jump root #'ripscope-find-definition "callee" "case.c" 3)
     (should (member "--no-auto-refresh" (car (ripscope-test-commands))))
     (with-current-buffer (ripscope-test-source root)
       (goto-char (point-max)) (insert "int ManualSaved;\n") (save-buffer))
     (should-error (ripscope-test-rows root 1 "ManualSaved") :type 'user-error)
     (with-current-buffer (ripscope-test-source root) (ripscope-index-project))
     (ripscope-test-jump root #'ripscope-find-definition "ManualSaved" "case.c" 5))
   ;; Back on, the next search starts a session that refreshes.
   (ripscope-test-rows root 1 "callee")
   (should (member "--auto-refresh" (car (ripscope-test-commands))))))

(ert-deftest ripscope-failures-and-warnings-are-reported ()
  (ripscope-test-project
   (let ((empty (should-error (ripscope-test-rows root 1 "Absent") :type 'user-error)))
     (should (string-match-p "found no definitions of Absent" (cadr empty))))
   (let ((pid (ripscope-test-pids))
         (invalid (should-error (ripscope-test-rows root 1 "[") :type 'user-error)))
     (should (string-match-p "invalid query pattern" (cadr invalid)))
     ;; The session answers again after it.
     (should (ripscope-test-rows root 1 "callee"))
     (should (equal (ripscope-test-pids) pid)))
   (should-error (ripscope-test-rows root 1 "two\nlines") :type 'user-error)
   (should-error (ripscope-test-rows root 1 "") :type 'user-error)
   ;; A text search reads the sources, and says which it could not read. Without
   ;; refreshing, the index still lists the removed header.
   (let ((ripscope-auto-refresh nil) rows)
     (delete-file "case.h")
     (should (string-match-p "Ripscope: cannot read .*case.h"
                             (ripscope-test-message
                              (lambda () (setq rows (ripscope-test-rows root 4 "value"))))))
     (should (equal (mapcar #'cadr rows) '(2 3 4)))
     (let ((empty (should-error (ripscope-test-rows root 4 "Absent") :type 'user-error)))
       (should (string-match-p "found no text Absent (cannot read .*case.h" (cadr empty))))
     (ripscope-test-write "case.h" "extern int value;\n")
     (should-not (string-match-p "cannot read"
                                 (ripscope-test-message
                                  (lambda () (ripscope-test-rows root 4 "value"))))))
   (let ((ripscope-executable (expand-file-name "missing/ripscope.exe" root)))
     (should-error (ripscope-test-rows root 1 "callee") :type 'user-error))))

(ert-deftest ripscope-large-answers ()
  (ripscope-test-project
   (ripscope-test-write "many.c" (mapconcat (lambda (n) (format "int many%d(void) { return value; }\n" n))
                                            (number-sequence 0 2499) ""))
   (let ((rows (ripscope-test-rows root 0 "value")))
     (should (= (length rows) 2504))
     (should (equal (nth 1 (car (last rows))) 2500)))))

(ert-deftest ripscope-sources-includes-and-file-lists ()
  (let ((shared (make-temp-file "ripscope shared é-" t)))
    (unwind-protect
        (ripscope-test-project
         (ripscope-test-write (expand-file-name "external.h" shared) "int External;\n")
         (ripscope-test-write "case.c" "#include <external.h>\nint Local;\n")
         (ripscope-test-write "other.c" "int Other;\n")
         (ripscope-test-write "cscope.files" "case.c\n")
         (let ((ripscope-include-directories (list shared)))
           (should (equal (ripscope-test-rows root 1 "External")
                          `((,(file-relative-name (expand-file-name "external.h" shared) root)
                             1 "int External;"))))
           (should-error (ripscope-test-rows root 1 "Other") :type 'user-error)
           (ripscope-test-write "cscope.files" "case.c\nother.c\n")
           (should (ripscope-test-rows root 1 "Other"))))
      (delete-directory shared t))))

(ert-deftest ripscope-a-buffer-without-a-file-searches-its-directory ()
  (ripscope-test-project
   (ripscope-test-rows root 1 "callee")
   (make-directory "other/.git" t)
   (ripscope-test-write "other/other.c" "int Elsewhere;\n")
   ;; As from Dired in another project, after a search in this one.
   (with-temp-buffer
     (setq default-directory (expand-file-name "other/" root))
     (should (equal (plist-get (ripscope--project) :root)
                    (directory-file-name (file-truename (expand-file-name "other" root))))))
   ;; The xref buffer keeps the project of the search that made it.
   (with-current-buffer (ripscope-test-source root) (ripscope-find-callers "callee"))
   (with-current-buffer "*xref*"
     (should (equal (plist-get (ripscope--project) :root) (file-truename root))))))

(ert-deftest ripscope-remote-files-are-left-to-other-backends ()
  (let ((default-directory "/ssh:example:/project/"))
    (should-not (ripscope--backend))
    (should-error (ripscope--project) :type 'user-error)))

(ert-deftest ripscope-package-compiles-and-passes-checkdoc ()
  (let* ((copy (make-temp-file "ripscope-compile-" t))
         (file (expand-file-name "ripscope.el" copy)))
    (unwind-protect
        (progn
          (copy-file ripscope-test-package file)
          (let ((byte-compile-error-on-warn t))
            (should (byte-compile-file file)))
          (let (problems)
            (cl-letf (((symbol-function 'checkdoc-create-error)
                       (lambda (text start &rest _)
                         (push (format "line %d: %s" (line-number-at-pos start) text) problems)
                         nil)))
              (with-current-buffer (find-file-noselect file)
                (checkdoc-current-buffer t)
                (kill-buffer)))
            (should-not problems))
          (let ((lint (getenv "RIPSCOPE_PACKAGE_LINT")))
            (if (not (and lint (not (string-empty-p lint)) (file-directory-p lint)))
                (princ "PACKAGE_LINT skipped\n")
              (add-to-list 'load-path lint)
              (require 'package-lint)
              (with-current-buffer (find-file-noselect file)
                (let ((issues (package-lint-buffer)))
                  (princ (format "PACKAGE_LINT %S\n" issues))
                  (should-not issues))
                (kill-buffer)))))
      (delete-directory copy t))))

(ert-deftest ripscope-session-failures ()
  :tags '(linux)
  (ripscope-test-project
   (ripscope-test-rows root 1 "callee")
   (dolist (case '(("printf '%s\\n' '{\"type\":\"result\",\"count\":2}' '[\"case.c\",\"x\",2,\"int value;\"]'\nprintf tail-diagnostic\nexit 7\n"
                    "stopped unexpectedly (exit 7): tail-diagnostic")
                   ("printf '%s\\n' '{\"type\":\"result\",\"count\":2}'\nprintf signal-tail\nkill -TERM $$\n"
                    "stopped unexpectedly (signal 15): signal-tail")
                   ("printf '%s\\n' '{\"type\":\"result\",\"count\":1}' '{\"type\":\"ready\"}'\nsleep 5\n"
                    "response it should not have")
                   ("sleep 5\n" "did not answer within 1 seconds")))
     (let ((fake (make-temp-file "ripscope-fake-backend-")))
       (unwind-protect
           (progn
             (ripscope-test-write
              fake (concat "#!/bin/sh\ncase \"$1\" in --project|-b) exec \"$RIPSCOPE_REAL\" \"$@\";; esac\n"
                           "printf '%s\\n' '{\"type\":\"ready\"}'\nread request\n" (car case)))
             (set-file-modes fake #o700)
             (let* ((ripscope-executable fake) (ripscope-timeout 1)
                    (process-environment (cons (concat "RIPSCOPE_REAL=" ripscope-test-backend)
                                               process-environment))
                    (failure (should-error (ripscope-test-rows root 1 "value"))))
               (should (string-match-p (regexp-quote (cadr case)) (cadr failure)))
               (should-not (cl-some (lambda (command) (equal (car command) fake))
                                    (ripscope-test-commands)))))
         (delete-file fake))))
   ;; The real session was never disturbed.
   (should (ripscope-test-rows root 1 "callee"))))

(when (eq system-type 'windows-nt)
  (load (expand-file-name "emacs-windows-workflow.el" (file-name-directory load-file-name)) nil t))
(ert-run-tests-batch-and-exit (if (eq system-type 'windows-nt) '(not (tag linux)) t))
