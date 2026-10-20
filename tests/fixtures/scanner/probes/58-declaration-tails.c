#define DECLARE(name, type)
#define STACK_OF(type) type
#define APIENTRYP *
#define PAD(n)
#define END

DECLARE(X, T)
DECLARE(Y, U)
int real(T *x, STACK_OF(Y) *items) {
    use_it(x);
}

static Object *(*get)(Other *value);
typedef void (APIENTRYP callback)(Number x, Number y);
typedef int (ABI query)(Other *value);

#if ENABLED
INIT
#else
int core;
#endif
;

struct ordinary {
    int member;
    PAD(5)
    END
};
struct single {
    int member;
    FIELDS
};
struct stacked {
    W1
    W2
    W3
};

typedef int T;
T
#if A
left
#else
right
#endif
;

struct split {
    T
    real_member
};

struct outer {
    enum inner
#if A
    {
#else
    {
#endif
        REAL_ENUM
    } member;
};

typedef TYPEOF(*ptr) (API alias);
typedef TYPEOF(left *right) (*alias);

FORCE_INLINE()
static int
actual(int x, UNUSED(int y))
DECOR ANNO(used(x)) { return x + y; }
