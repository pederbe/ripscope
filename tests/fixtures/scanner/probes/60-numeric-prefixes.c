static FORMAT(2, 3) void logging(Context *ctx, const char *fmt, ...)
{
    send(ctx);
}
PREFIX(1) /* annotation payload, then a written return type */
int annotated(void) { target(); }

int ordinary(int value) { target(); }
old_style(value)
int value;
{ target(); }
DIRECT(1) { target(); }
PLAIN_NAMES(value) int following(void) { target(); }
TYPE_NAME(1) Result alias_result(void) { target(); }
CONDITIONAL(1)
#if ENABLED
int conditional(void) { target(); }
#endif
