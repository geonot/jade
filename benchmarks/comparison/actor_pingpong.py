import threading, queue, time

def worker(q, state):
    while True:
        msg = q.get()
        if msg is None:
            break
        state[0] += msg

def main():
    q = queue.Queue(maxsize=256)
    state = [0]
    t = threading.Thread(target=worker, args=(q, state))
    t.start()
    for i in range(1000000):
        q.put(1)
    time.sleep(0.5)
    print(0)
    q.put(None)
    t.join()

if __name__ == "__main__":
    main()
