;;; setup.el --- One-step setup check -*- lexical-binding: t; no-byte-compile: t -*-
;; Run by check-editor-setup in a fresh profile: evaluate the init line a user adds,
;; open a C file, search with M-., save an edit and search again.
(require 'json)
(require 'xref)
;; Version control stays on: package-vc clones with it.
(setq make-backup-files nil auto-save-default nil create-lockfiles nil)
(defvar setup-errors nil)
(defvar setup-searches nil)

(defun setup-wait (predicate seconds)
  (let ((deadline (+ (float-time) seconds)))
    (while (and (not (funcall predicate)) (< (float-time) deadline))
      (accept-process-output nil 0.05))
    (unless (funcall predicate) (error "Timed out after %s seconds" seconds))))

(defun setup-search (file pattern)
  (let ((source (find-file-noselect file)) rows jump output)
    (switch-to-buffer source)
    (unless (bound-and-true-p ripscope-mode)
      (push (format "ripscope-mode is off in %s" file) setup-errors))
    ;; The definitions M-. finds, then M-. itself.
    (condition-case problem
        (dolist (item (xref-backend-definitions (xref-find-backend) pattern))
          (let ((location (xref-item-location item)))
            (push `((file . ,(expand-file-name (xref-location-group location)))
                    (line . ,(xref-location-line location)))
                  rows)))
      (user-error (setq output (error-message-string problem))))
    (when rows
      (with-current-buffer source
        (xref-find-definitions pattern))
      (with-current-buffer (window-buffer (selected-window))
        (goto-char (window-point))
        (setq jump `((file . ,(expand-file-name (buffer-file-name)))
                     (line . ,(line-number-at-pos))
                     (text . ,(buffer-substring-no-properties
                               (line-beginning-position) (line-end-position)))))))
    (push `((pattern . ,pattern) (rows . ,(vconcat (nreverse rows)))
            (jump . ,(or jump :json-null)) (output . ,(or output "")))
          setup-searches)))

(condition-case problem
    (let ((file (getenv "RIPSCOPE_SETUP_FILE"))
          (absent (getenv "RIPSCOPE_SETUP_ABSENT")))
      (eval (car (read-from-string (concat "(progn " (getenv "RIPSCOPE_SETUP_INIT") ")"))) t)
      ;; ripscope-setup installs a missing executable in the background.
      (when (bound-and-true-p ripscope--installation)
        (setup-wait (lambda () (not (process-live-p ripscope--installation))) 120)
        (accept-process-output nil 0.2))
      (setup-search file "greet")
      (when (and absent (not (string-empty-p absent))) (setup-search file absent))
      (with-current-buffer (find-file-noselect file)
        (goto-char (point-max))
        (insert "int fresh_symbol(void) { return 3; }\n")
        (save-buffer))
      (setup-search file "fresh_symbol"))
  (error (push (error-message-string problem) setup-errors)))
(with-temp-file (getenv "RIPSCOPE_SETUP_REPORT")
  (insert (json-encode
           `((errors . ,(vconcat setup-errors))
             (searches . ,(vconcat (nreverse setup-searches)))
             (installer_started . ,(if (bound-and-true-p ripscope--installation) t :json-false))
             (installer . ,(if (get-buffer "*ripscope-install*")
                               (with-current-buffer "*ripscope-install*" (buffer-string))
                             ""))
             (indexing . ,(if (get-buffer "*ripscope-index*")
                              (with-current-buffer "*ripscope-index*" (buffer-string))
                            ""))))))
(kill-emacs 0)
