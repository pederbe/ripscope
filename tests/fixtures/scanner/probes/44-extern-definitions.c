extern void first(int x)
{
  use(x);
}
#define LOCAL int y = 0; (void)y
typedef struct {
  const char* start;
  const char* end;
} span;
int after;
extern int second(void) { return 0; }
int lost_by_cscope;
int kept;
extern int declared;
int defined_after_declaration;
