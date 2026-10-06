class Vector3:
    __slots__ = ('x', 'y', 'z')
    def __init__(self, x=0, y=0, z=0):
        self.x = x
        self.y = y
        self.z = z

class Node:
    __slots__ = ('val', 'next')
    def __init__(self, val=0, next=None):
        self.val = val
        self.next = next

def run():
    # 1. Vector3
    v = Vector3(10, 20, 30)
    v_sum = v.x + v.y + v.z

    # 2. Linked list: 3 nodes
    node3 = Node(300, None)
    node2 = Node(200, node3)
    node1 = Node(100, node2)

    list_sum = 0
    curr = node1
    while curr is not None:
        list_sum += curr.val
        curr = curr.next
    return (v_sum, list_sum)

if __name__ == '__main__':
    for _ in range(100_000):
        run()
