/* K&R heads whose identifier list ends with a parameter macro, and whose
   declarations end with a declaration macro before the body. */
static char *
lower_copy (dest, src, len EXTRA_PARAM)
     char *dest;
     const char *src;
     int len;
     EXTRA_PARAM_DECL
{
  while (len-- > 0)
    dest[len] = to_lower (src[len]);
  return dest;
}

int
count_items (list, n extra_argument EXTRA_PARAM)
     struct item *list;
     int n;
     extra_argument_spec
{
  return walk_items (list, n);
}

/* A prototype of the same written shape has no body. */
int declared_only (size_t n);
EXTRA_PARAM_DECL

int after_all (int x)
{
  return helper (x);
}
