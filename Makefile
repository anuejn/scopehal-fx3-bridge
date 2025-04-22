CFLAGS=-g -Wall -Wextra $(shell pkg-config --cflags libusb-1.0)

scopehal-fx-bridge: scopehal-fx-bridge.o ezusb.o
	$(CC) -o scopehal-fx-bridge $^ $(shell pkg-config --libs libusb-1.0)

fxload: ezusb.o fxload.o
	$(CC) -o fxload $^ $(shell pkg-config --libs libusb-1.0)

clean:
	rm *.o fxload