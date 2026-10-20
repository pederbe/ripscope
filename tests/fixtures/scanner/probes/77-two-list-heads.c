/* A macro's argument list followed by the parameter list of the function it heads. */
DEFUN ("make-widget", Fmake_widget, Smake_widget,
       1, 2, 0,
       doc: /* Make a widget.  */)
  (Lisp_Object spec, Lisp_Object parent)
{
  Lisp_Object widget = build_widget (spec);
  attach_widget (widget, parent);
  return widget;
}

/* One identifier in the first list. */
static int
PRIV(check_escape)(const char *text, int length)
{
  return scan_escape (text, length);
}

int after_heads (void)
{
  return helper (0);
}
