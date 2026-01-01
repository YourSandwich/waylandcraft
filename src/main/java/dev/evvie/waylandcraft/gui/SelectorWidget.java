package dev.evvie.waylandcraft.gui;

import java.util.stream.Stream;

import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.narration.NarrationElementOutput;
import net.minecraft.network.chat.Component;

public abstract class SelectorWidget<T> extends AbstractWidget {
	
	private SelectorButton<T>[] buttons;
	private int count = 0;
	
	// Currently selected element, should always be either null or an element assigned to a button
	private T selected = null;
	
	public SelectorWidget(int x, int y, int buttonWidth, int buttonHeight, int maxCount) {
		super(x, y, buttonWidth * maxCount, buttonHeight, Component.empty());
		
		if(maxCount < 1) throw new IllegalArgumentException("SelectorWidget maxCount < 1");
		
		buttons = new SelectorButton[maxCount];
		for(int i = 0; i < buttons.length; i++) {
			buttons[i] = new SelectorButton<T>(this, x + buttonWidth * i, y, buttonWidth, buttonHeight, i);
		}
	}
	
	public void setEntries(T[] entries) {
		count = Math.min(entries.length, buttons.length);
		
		for(int i = 0; i < count; i++) {
			buttons[i].element = entries[i];
			buttons[i].setMessage(titleForElement(buttons[i].element));
		}
		
		if(entries.length == 0) {
			buttons[0].setMessage(Component.empty());
			buttons[0].element = null;
			count = 1;
		}
		
		selectionCheck();
	}
	
	public abstract Component titleForElement(T element);
	
	public T selection() {
		return selected;
	}
	
	// Maintains selected element property
	private void selectionCheck() {
		if(!Stream.of(buttons).anyMatch((b) -> b.element == selected)) {
			selected = null;
		}
	}
	
	public void select(T element) {
		this.selected = element;
		selectionCheck();
	}
	
	@Override
	protected void renderWidget(GuiGraphics guiGraphics, int mouseX, int mouseY, float partialTicks) {
		for(int i = 0; i < buttons.length; i++) {
			SelectorButton<T> b = buttons[i];
			b.selected = b.element == selected;
			b.visible = i < count;
			
			b.render(guiGraphics, mouseX, mouseY, partialTicks);
		}
	}
	
	@Override
	public boolean mouseClicked(double x, double y, int mouseButton) {
		if(!(this.active && this.visible)) return false;
		
		for(SelectorButton<T> b : buttons) {
			if(b.mouseClicked(x, y, mouseButton)) return true;
		}
		
		return false;
	}
	
	@Override
	protected void updateWidgetNarration(NarrationElementOutput narrationElementOutput) {
	}
	
	private static class SelectorButton<T> extends Button {
		
		public T element = null;
		public boolean selected = false;
		
		public SelectorButton(SelectorWidget<T> widget, int x, int y, int width, int height, int idx) {
			super(x, y, width, height, Component.empty(), (b) -> {widget.select(((SelectorButton<T>) b).element);}, (c) -> c.get());
		}
		
		@Override
		public boolean isHoveredOrFocused() {
			// Lazy hack for rendering
			return selected;
		}
		
	}
	
}
