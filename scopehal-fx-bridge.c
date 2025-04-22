#include <stdlib.h>
#include "libusb.h"


int main(int argc, char*argv[]) {

}

int upload_firmware() {
	libusb_device *dev, **devs;

	if (libusb_get_device_list(NULL, &devs) < 0) {
		logerror("libusb_get_device_list() failed\n");
		return -1;
	}
	for (int i=0; (dev=devs[i]) != NULL; i++) {
		unsigned int _busnum = libusb_get_bus_number(dev);
		unsigned int _devaddr = libusb_get_device_address(dev);
		struct libusb_device_descriptor desc;
		int status = libusb_get_device_descriptor(dev, &desc);
		if (status >= 0) {
			if ((desc.idVendor == known_device[j].vid) && (desc.idProduct == known_device[j].pid)) {
				if (// nothing was specified
					((type == NULL) && (device_id == NULL) && (device_path == NULL)) ||
					// vid:pid was specified and we have a match
					((type == NULL) && (device_id != NULL) && (vid == desc.idVendor) && (pid == desc.idProduct)) ||
					// bus,addr was specified and we have a match
					((type == NULL) && (device_path != NULL) && (busnum == _busnum) && (devaddr == _devaddr)) ||
					// type was specified and we have a match
					((type != NULL) && (device_id == NULL) && (device_path == NULL) && (fx_type == known_device[j].type)) ) {
					fx_type = known_device[j].type;
					vid = desc.idVendor;
					pid = desc.idProduct;
					busnum = _busnum;
					devaddr = _devaddr;
					break;
				}
			}
		}
		
	}
	if (dev == NULL) {
		libusb_free_device_list(devs, 1);
		libusb_exit(NULL);
		logerror("could not find a known device - please specify type and/or vid:pid and/or bus,dev\n");
		return print_usage(-1);
	}
	status = libusb_open(dev, &device);
	libusb_free_device_list(devs, 1);
	if (status < 0) {
		logerror("libusb_open() failed: %s\n", libusb_error_name(status));
		goto err;
	}
}