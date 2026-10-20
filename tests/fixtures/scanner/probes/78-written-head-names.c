/* A function head named by a macro of one identifier, followed by its own list. */
static int
PRIV(check_escape)(const char *text, int length)
{
  return scan_escape (text, length);
}

/* Macros with several arguments, a keyword argument and a list that is not followed by
   a second list keep the reference's reading. */
SYSCALL_DEFINE2(open, const char *, path, int, flags)
{
  return do_open (path, flags);
}

static int
WRAP(int)(const char *text)
{
  return count_text (text);
}

void caller (void)
{
  PRIV(check_escape)("x", 1);
}
