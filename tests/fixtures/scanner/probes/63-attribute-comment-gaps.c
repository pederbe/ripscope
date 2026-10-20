int annotated(void) __attribute__ /* gap */ ((unused)) { target(); return 0; }
int expression(void) __attribute__ /* gap */ ((annotate(helper()))) { target(); return 0; }
struct __attribute__ /* gap */ ((aligned(8))) item { int member; };
__declspec /* gap */ (align(8)) int global;
int after;
int synonym(void) __attribute /* gap */ ((unused)) { target(); return 0; }
struct __attribute__ /* one */ /* two */ ((packed)) chained { int member; };
int prototype(void) __attribute__ /* gap */ ((unused));
#define EXPR __attribute__ /* gap */ ((annotate(macro_helper())))
int closed(void) __attribute__ /* two
lines */ ((unused)) { target(); return 0; }
int spliced(void) __attribute__ /* continued \
*/ ((unused)) { target(); return 0; }
int unterminated(void) __attribute__ /* unfinished
