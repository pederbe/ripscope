;;; emacs-windows-workflow.el --- Windows workflows -*- lexical-binding: t; no-byte-compile: t -*-
;; Loaded by emacs-workflow.el, which supplies the shared project and search helpers.

(ert-deftest ripscope-windows-backend-path-preserves-the-staged-accent ()
  (should (string-suffix-p
           "/backend & percent% é/ripscope.exe"
           (subst-char-in-string ?\\ ?/ (ripscope--executable))))
  (should (file-executable-p (ripscope--executable))))

(ert-deftest ripscope-windows-searches-pass-any-character ()
  ;; Searches reach the session as UTF-8, whatever the ANSI code page holds.
  (ripscope-test-project
   (ripscope-test-write "case.c" "// café & percent%\nint Local;\n")
   (let ((empty (should-error (ripscope-test-rows root 4 (string #x10000)) :type 'user-error)))
     (should (string-match-p "found no text" (cadr empty))))
   (should (equal (mapcar #'cadr (ripscope-test-rows root 4 "café & percent%")) '(1)))))

(ert-deftest ripscope-windows-refuses-a-shell-script-backend ()
  (let ((ripscope-executable (expand-file-name "ripscope.cmd" temporary-file-directory)))
    (ripscope-test-write ripscope-executable "@echo off\n")
    (unwind-protect
        (should-error (ripscope--executable) :type 'user-error)
      (delete-file ripscope-executable))))
