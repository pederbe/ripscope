#define BAD don't use this
int after_bad;
#define OPEN "unfinished
int after_open;
#define CLOSED 'x'
int after_closed;
#define CONTINUED 'a\
b'
int after_continued;
struct state {
#define MEMBER don't
    int member;
};
int function(void) {
#define LOCAL "unfinished
    target();
}
int after_function;
#if ENABLE
#define CONDITIONAL don't
#endif
int after_conditional;
#define TWO "slash \\
ncontinued"
int after_two;
#define FOUR "slash \\\\
fcontinued"
int after_four;
#define ESCAPED_TWO "\\"
int after_escaped_two;
#define ESCAPED_FOUR "\\\\"
int after_escaped_four;
