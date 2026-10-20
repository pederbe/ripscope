typedef int Count;
typedef void (*Callback)(int);
struct Item { int value; Callback run; };
union Storage { int number; char byte; };
enum Mode { IDLE, BUSY = 2 };
int global = 1;
extern int external;
void declared(int);
void (*factory(void))(int) { return 0; }
void ordinary(void) { int local = 0; local += global; }
