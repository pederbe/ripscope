/* Unicode before names: æ漢 */
#define SIZE 4
#define DECL(name) int name
typedef int Count;
typedef void (*Callback)(int);
struct Item { int value; Callback run; };
union Storage { int number; char byte; };
enum Mode { IDLE, BUSY = 2 };
int global = 1, spare;
extern int external;
extern int initialized_external = 2;
void declared(int);
int *pointer_result(void);
int (*function_pointer)(void);
int (*pointer_array[2])(void);
void (*factory(void))(int) { return 0; }
void ordinary(void) { int local = 0; { int local = 1; } }
