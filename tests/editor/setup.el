;;; setup.el --- One-step setup check -*- lexical-binding: t; no-byte-compile: t -*-
;; Run by check-editor-setup in a fresh profile: evaluate the init line a user adds,
;; open a C file, search, save an edit and search again.
(require 'json)
;; Version control stays on: package-vc clones with it.
(setq make-backup-files nil auto-save-default nil create-lockfiles nil)
(defvar setup-errors nil)
(defvar setup-searches nil)

(defun setup-wait (predicate seconds)
  (let ((deadline (+ (float-time) seconds)))
    (while (and (not (funcall predicate)) (< (float-time) deadline))
      (accept-process-output nil 0.05))
    (unless (funcall predicate) (error "Timed out after %s seconds" seconds))))

(defun setup-row (position)
  (let ((props (cscope-get-navigation-properties position)))
    (when (and (elt props 0) (elt props 1))
      `((file . ,(expand-file-name (elt props 0))) (line . ,(elt props 1))))))

(defun setup-search (file pattern)
  (with-current-buffer (find-file-noselect file)
    (cscope-find-global-definition pattern))
  (let (rows jump)
    (with-current-buffer cscope-output-buffer-name
      (setup-wait (lambda () (null cscope-process)) 90)
      (save-excursion
        (goto-char cscope-output-start)
        (while (not (eobp))
          (let ((row (setup-row (point))))
            (when row (push row rows)))
          (forward-line 1)))
      (when cscope-first-match-point
        (let ((window (cscope-show-entry-internal
                       (cscope-get-navigation-properties cscope-first-match-point))))
          (with-current-buffer (window-buffer window)
            (goto-char (window-point window))
            (setq jump `((file . ,(expand-file-name (buffer-file-name)))
                         (line . ,(line-number-at-pos))
                         (text . ,(buffer-substring-no-properties
                                   (line-beginning-position) (line-end-position)))))))))
    (push `((pattern . ,pattern) (rows . ,(vconcat (nreverse rows)))
            (jump . ,(or jump :json-null))
            (output . ,(with-current-buffer cscope-output-buffer-name
                         (buffer-substring-no-properties cscope-output-start (point-max)))))
          setup-searches)))

(condition-case problem
    (let ((file (getenv "RIPSCOPE_SETUP_FILE"))
          (absent (getenv "RIPSCOPE_SETUP_ABSENT")))
      (eval (car (read-from-string (concat "(progn " (getenv "RIPSCOPE_SETUP_INIT") ")"))) t)
      ;; ripscope-setup installs a missing executable in the background.
      (when (bound-and-true-p ripscope--installation)
        (setup-wait (lambda () (not (process-live-p ripscope--installation))) 120)
        (accept-process-output nil 0.2))
      (setq cscope-display-times nil cscope-edit-single-match nil)
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
             (indexing . ,(if (get-buffer "*cscope-indexing-buffer*")
                              (with-current-buffer "*cscope-indexing-buffer*" (buffer-string))
                            ""))))))
(kill-emacs 0)
