extern int ext_var;
extern int ext_func(int);
int defined_after_extern;
typedef int (*callback_t)(void *ctx);
typedef struct node { struct node *next; int key; } node_t, *node_ptr;
typedef enum { OFF, ON } state_t;
typedef unsigned char byte;
byte buffer[16];
