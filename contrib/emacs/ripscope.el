;;; ripscope.el --- Fast cscope searches for C, through xcscope -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-2.0-or-later
;; Version: 0.1.0
;; Package-Requires: ((emacs "27.1"))
;; Keywords: c, languages, tools
;; URL: https://github.com/pederbe/ripscope

;;; Commentary:
;; Ripscope answers cscope's searches for C.  After `ripscope-setup',
;; xcscope's usual keys (C-c s g for a definition, C-c s c for callers
;; and the rest) search with it in any C project, with no configuration:
;; the first search in a project indexes it into your cache directory,
;; and later searches see your saved edits.
;;
;; With use-package on Emacs 30 or newer, in one line:
;;
;;   (use-package ripscope
;;     :vc (:url "https://github.com/pederbe/ripscope" :branch "release"
;;          :lisp-dir "contrib/emacs" :rev :newest)
;;     :config (ripscope-setup))
;;
;; See README.md beside this file for the details and other ways to set
;; it up.

;;; Code:

(require 'subr-x)

(declare-function ripscope-xcscope--enable "ripscope-xcscope" (backend automatic))
(declare-function ripscope-xcscope--find-executable "ripscope-xcscope" ())
(declare-function ripscope-xcscope--use "ripscope-xcscope" (backend))
(defvar ripscope-xcscope--automatic)

(defconst ripscope--directory
  (file-name-directory (or load-file-name buffer-file-name))
  "The directory of this file and the rest of the integration.")

(defconst ripscope--repository
  (let ((root (expand-file-name "../.." ripscope--directory)))
    ;; Only a checkout, as plugin managers clone the repository, holds this file in
    ;; its contrib/emacs. A copy the installer placed has no checkout around it.
    (and (file-equal-p (expand-file-name "contrib/emacs" root) ripscope--directory)
         (file-regular-p (expand-file-name "Cargo.toml" root))
         root))
  "The Ripscope checkout this copy belongs to, or nil when it is not in one.")

(defun ripscope--load ()
  "Load the xcscope integration beside this file, whatever `load-path' is."
  (require 'ripscope-xcscope (expand-file-name "ripscope-xcscope" ripscope--directory)))

(defvar ripscope--installation nil
  "The running installer, if any.")

(defun ripscope--installer ()
  "This checkout's installer script, or nil when the copy is not in a checkout."
  (when ripscope--repository
    (let ((script (expand-file-name (if (eq system-type 'windows-nt) "install.ps1" "install.sh")
                                    ripscope--repository)))
      (and (file-regular-p script) script))))

(defun ripscope--checkout-version ()
  "The version of the Ripscope repository this copy came from, or nil."
  (let ((manifest (and ripscope--repository
                       (expand-file-name "Cargo.toml" ripscope--repository))))
    (when (and manifest (file-readable-p manifest))
      (with-temp-buffer
        (insert-file-contents manifest)
        (when (re-search-forward "^version = \"\\([^\"]+\\)\"" nil t)
          (match-string 1))))))

(defun ripscope--installed-version (executable)
  "The version EXECUTABLE reports, or nil."
  (let ((line (car (ignore-errors (process-lines executable "--version")))))
    (and line (string-match "\\`ripscope \\(\\S-+\\)" line) (match-string 1 line))))

(defun ripscope--installer-executable ()
  "Where Ripscope's installer puts the executable."
  (expand-file-name (if (eq system-type 'windows-nt) "ripscope.exe" "ripscope")
                    (or (getenv "RIPSCOPE_INSTALL_DIR")
                        (expand-file-name ".local/bin"
                                          (or (and (eq system-type 'windows-nt)
                                                   (getenv "USERPROFILE"))
                                              "~")))))

(defun ripscope--wanted-p (found)
  "Whether to install over the executable FOUND, which may be nil: it is
missing, or the installer put it there and it is older than this copy."
  (let ((wanted (ripscope--checkout-version)))
    (or (null found)
        (and wanted
             (file-equal-p found (ripscope--installer-executable))
             (let ((have (ripscope--installed-version found)))
               (or (null have) (version< have wanted)))))))

;;;###autoload
(defun ripscope-setup (&optional executable)
  "Search C projects with Ripscope through xcscope's usual keys.
Where xcscope finds no cscope database of its own, a search's project
is the nearest directory upward holding cscope.files, else the nearest
Git repository, else the search's directory.  The first search there
indexes it into your cache directory.

EXECUTABLE is the absolute path of the ripscope executable.  By default
it is found on PATH or where Ripscope's installer puts it, and
`ripscope-install' installs it when it is missing or older than this
copy.  Call this once, before any other copy of xcscope loads."
  (ripscope--load)
  (ripscope-xcscope--enable executable t)
  (unless executable
    (let ((found (ripscope-xcscope--find-executable)))
      (cond
       ((not (ripscope--wanted-p found)))
       ((ripscope--installer) (ripscope-install))
       ((null found)
        ;; A message, not an error, so that the rest of the init file still loads.
        (message "Ripscope: the ripscope executable is not installed; %s"
                 "run the one-line installer from https://ripscope.com"))))))

(defun ripscope--powershell ()
  (expand-file-name "System32/WindowsPowerShell/v1.0/powershell.exe"
                    (or (getenv "SystemRoot") "C:/Windows")))

;;;###autoload
(defun ripscope-install (&optional force)
  "Install the ripscope executable with Ripscope's verifying installer.
Installs this copy's version when the executable is missing or older,
or anyway with FORCE, the prefix argument.  The installer runs in the
background and reports in the *ripscope-install* buffer."
  (interactive "P")
  (ripscope--load)
  (let* ((windows (eq system-type 'windows-nt))
         (found (ripscope-xcscope--find-executable))
         (wanted (ripscope--checkout-version))
         (script (ripscope--installer)))
    (cond
     ((and (not force) (not (ripscope--wanted-p found)))
      (message "Ripscope %s is installed: %s" (ripscope--installed-version found) found))
     ((process-live-p ripscope--installation)
      (message "Ripscope's installer is running already"))
     ((not script)
      (user-error "This copy of Ripscope has no installer; run the one-line installer from https://ripscope.com"))
     (t
      (let ((default-directory (file-name-as-directory ripscope--repository))
            ;; The integration is here already, so the installer leaves the editors alone.
            (process-environment
             (append (list "RIPSCOPE_EDITORS=none")
                     (and wanted (list (concat "RIPSCOPE_VERSION=" wanted)))
                     process-environment))
            (buffer (get-buffer-create "*ripscope-install*")))
        (with-current-buffer buffer
          (let ((inhibit-read-only t)) (erase-buffer)))
        (message "Ripscope: installing ripscope %s..." (or wanted ""))
        (setq ripscope--installation
              (make-process
               :name "ripscope-install" :buffer buffer :noquery t
               :command (if windows
                            (list (ripscope--powershell) "-NoProfile" "-NonInteractive"
                                  "-ExecutionPolicy" "Bypass" "-File" script)
                          (list "sh" script))
               :sentinel #'ripscope--installed)))))))

(defun ripscope--installed (process _event)
  "Report how PROCESS, the installer, went."
  (unless (process-live-p process)
    (let ((found (ripscope-xcscope--find-executable)))
      (if (and found (eq (process-status process) 'exit) (= (process-exit-status process) 0))
          (progn
            (when ripscope-xcscope--automatic (ripscope-xcscope--use found))
            (message "Ripscope: installed %s" found))
        (message "Ripscope: installation failed; see %s" (buffer-name (process-buffer process)))))))

(provide 'ripscope)
;;; ripscope.el ends here
