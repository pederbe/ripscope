;;; emacs-windows-workflow.el --- Native discovery workflows -*- lexical-binding: t; no-byte-compile: t -*-
;; Loaded by emacs-workflow.el, which supplies the shared project and query helpers.

(ert-deftest ripscope-windows-backend-path-preserves-the-staged-accent ()
  (should (string-suffix-p
           "/backend & percent% é/ripscope.exe"
           (subst-char-in-string ?\\ ?/ ripscope-xcscope--backend)))
  (should (file-executable-p ripscope-xcscope--backend)))

(ert-deftest ripscope-windows-discovers-without-shell-helpers ()
  (dolist (helper '("bash" "find" "sed")) (should-not (executable-find helper)))
  (ripscope-test-project
   (should (= (car (ripscope-test-build root)) 0))
   (should-not (file-exists-p "cscope.files"))
   (make-directory "nested")
   (with-temp-file "nested/new.c" (insert "int Added;\n"))
   (should (= (car (ripscope-test-build root)) 0))
   (ripscope-test-query root 'cscope-find-global-definition "Added")
   (should (ripscope-test-match))
   (let ((cscope-index-recursively nil))
     (should (= (car (ripscope-test-build root)) 0)))
   (ripscope-test-query root 'cscope-find-global-definition "Added")
   (should-not (ripscope-test-match))
   (should (= (car (ripscope-test-build root)) 0))
   (delete-file "nested/new.c")
   (should (= (car (ripscope-test-build root)) 0))
   (ripscope-test-query root 'cscope-find-global-definition "Added")
   (should-not (ripscope-test-match))))

(ert-deftest ripscope-windows-preserves-explicit-selection ()
  (ripscope-test-project
   (with-temp-file "other.c" (insert "int Other;\n"))
   (with-temp-file "cscope.files" (insert "case.c\n"))
   (let ((selection (ripscope-test-bytes "cscope.files")))
     (should (= (car (ripscope-test-build root)) 0))
     (should (equal selection (ripscope-test-bytes "cscope.files")))
     (should-error (cscope-create-list-of-files-to-index root) :type 'user-error)
     (should (equal selection (ripscope-test-bytes "cscope.files"))))
   (ripscope-test-query root 'cscope-find-global-definition "Other")
   (should-not (ripscope-test-match))
   (with-temp-file "custom.files" (insert "other.c\n"))
   (let ((cscope-index-file "custom.files"))
     (should (= (car (ripscope-test-build root)) 0))
     (delete-file "custom.files")
     (let ((index (ripscope-test-bytes "cscope.out"))
           (result (ripscope-test-build root)))
       (should-not (= (car result) 0))
       (should-not (string-match-p "Indexing finished" (cadr result)))
       (should (equal index (ripscope-test-bytes "cscope.out")))))
   (let ((ripscope-xcscope-auto-refresh nil))
     (ripscope-test-query root 'cscope-find-global-definition "Other")
     (should (ripscope-test-match)))))

(ert-deftest ripscope-windows-includes-and-native-argument-coding ()
  (ripscope-test-project
   (make-directory "include-é")
   (with-temp-file "include-é/external.h" (insert "int External;\n"))
   (with-temp-file "case.c" (insert "#include <external.h>\n// café & percent%\nint Local;\n"))
   (let ((ripscope-xcscope-include-directories '("include-é"))
         (cscope-index-recursively nil)
         (cscope-database-file "index é & percent%.db"))
     (should (= (car (ripscope-test-build root)) 0))
     (should (file-exists-p cscope-database-file))
     (ripscope-test-query root 'cscope-find-global-definition "External")
     (should (ripscope-test-match))
     (let ((text (ripscope-test-query root 'cscope-find-this-text-string "café & percent%")))
       (should (ripscope-test-match))
       (should (string-match-p "café & percent%" text))))))

(ert-deftest ripscope-windows-concurrent-build-is-refused ()
  (ripscope-test-project
   (should (= (car (ripscope-test-build root)) 0))
   (let* ((process-connection-type nil)
          (process (start-process "held-reader" nil ripscope-xcscope--backend "-dl" "-f" "cscope.out"))
          (cscope-unix-index-process process))
     (unwind-protect
         (progn (should-error (cscope-index-files root) :type 'user-error)
                (should (eq process cscope-unix-index-process)))
       (delete-process process)))))

(ert-deftest ripscope-windows-searches-pass-any-character ()
  ;; Searches reach the session as UTF-8, whatever the ANSI code page holds.
  (ripscope-test-project
   (should (= (car (ripscope-test-build root)) 0))
   (let ((text (ripscope-test-query root 'cscope-find-this-text-string (string #x10000))))
     (should (string-match-p "No matches were found" text))
     (should (string-match-p "Search complete" text)))
   (ripscope-test-query root 'cscope-find-global-definition "callee")
   (should (ripscope-test-match))))
