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

(defun ripscope--manifest ()
  "This checkout and the SHA-256 of its Cargo.toml, which names its version."
  (with-temp-buffer
    (insert-file-contents (expand-file-name "Cargo.toml" ripscope--repository))
    (concat ripscope--repository "\n" (secure-hash 'sha256 (current-buffer)))))

(defun ripscope--checked-file ()
  "Where the manifest the installer last checked the executable for is kept."
  (locate-user-emacs-file "ripscope-installer-checked"))

(defun ripscope--changed-p (found)
  "Whether the installer should look at FOUND, the executable or nil: it is
missing, or the installer has not checked it for this checkout's manifest,
as after an upgrade of the package.  Starting Emacs then runs no installer."
  (or (null found)
      (not (equal (ignore-errors
                    (with-temp-buffer
                      (insert-file-contents (ripscope--checked-file))
                      (buffer-string)))
                  (ripscope--manifest)))))

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
       ((ripscope--installer)
        (when (ripscope--changed-p found) (ripscope-install)))
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
It installs this copy's version, but leaves an executable of that version
or newer alone, and one it did not install, unless FORCE, the prefix
argument, is given.  The installer runs in the background and reports in
the *ripscope-install* buffer."
  (interactive "P")
  (ripscope--load)
  (let ((windows (eq system-type 'windows-nt))
        (script (ripscope--installer))
        (quiet (not (called-interactively-p 'any))))
    (cond
     ((process-live-p ripscope--installation)
      (message "Ripscope's installer is running already"))
     ((not script)
      (user-error "This copy of Ripscope has no installer; run the one-line installer from https://ripscope.com"))
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
    (let ((found (ripscope-xcscope--find-executable))
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
            (when ripscope-xcscope--automatic (ripscope-xcscope--use found))
            (ignore-errors
              (let ((checked (ripscope--checked-file)))
                (make-directory (file-name-directory checked) t)
                (with-temp-file checked (insert (ripscope--manifest)))))
            (when (or (not (process-get process 'quiet))
                      (and said (string-prefix-p "Installed" said)))
              (message "Ripscope: %s" (or said (format "installed %s" found)))))
        (message "Ripscope: installation failed; see %s" (buffer-name (process-buffer process)))))))

(provide 'ripscope)
;;; ripscope.el ends here
